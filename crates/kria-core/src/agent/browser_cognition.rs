//! Browser Cognition Engine
//!
//! Provides semantic browser automation beyond simple URL navigation.
//! Uses Chrome DevTools Protocol (CDP) natively via Rust WebSockets.
//!
//! ## Capabilities
//! - Page state reading (title, URL, content)
//! - Form filling via CDP
//! - Link/button clicking via CDP
//! - Tab management
//! - Download handling
//! - Auth flow detection
//!
//! ## Architecture
//! CDP is accessed directly via `reqwest` for tab discovery and `tokio-tungstenite`
//! for WebSocket communication. This replaces the brittle Python subprocess approach
//! and removes the dependency on the Python `websocket-client` package.

use futures::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;

static MANAGED_BROWSER_PID: AtomicU32 = AtomicU32::new(0);
static MANAGED_TARGET_ID: RwLock<Option<String>> = RwLock::new(None);

/// Current state of the browser.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BrowserState {
    /// Current page URL
    pub url: String,
    /// Current page title
    pub title: String,
    /// Whether the page is loading
    pub loading: bool,
    /// Active tab index
    pub active_tab: usize,
    /// Total number of tabs
    pub tab_count: usize,
    /// Whether a dialog/popup is visible
    pub dialog_visible: bool,
    /// Dialog message if visible
    pub dialog_message: Option<String>,
}

impl BrowserState {
    pub fn unknown() -> Self {
        Self {
            url: String::new(),
            title: String::new(),
            loading: false,
            active_tab: 0,
            tab_count: 0,
            dialog_visible: false,
            dialog_message: None,
        }
    }
}

/// Result of a browser operation.
#[derive(Debug, Clone)]
pub struct BrowserResult {
    pub success: bool,
    pub evidence: String,
    pub data: Option<serde_json::Value>,
}

impl BrowserResult {
    pub fn ok(evidence: impl Into<String>) -> Self {
        Self {
            success: true,
            evidence: evidence.into(),
            data: None,
        }
    }
    pub fn ok_with_data(evidence: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            success: true,
            evidence: evidence.into(),
            data: Some(data),
        }
    }
    pub fn err(evidence: impl Into<String>) -> Self {
        Self {
            success: false,
            evidence: evidence.into(),
            data: None,
        }
    }
}

/// Browser cognition engine using native CDP.
///
/// # Batch 1: PSDG persistence
///
/// Attach a `PsdgHandle` via `with_world_model()` to persist `BrowserState`
/// (URL, title) to WorldModelStore after each `get_state()` call.
/// All persistence is fire-and-forget — failures are silently logged.
pub struct BrowserCognitionEngine {
    /// Optional PSDG handle for browser state persistence.
    world_model: Option<crate::agent::psdg::PsdgHandle>,
}

impl BrowserCognitionEngine {
    pub fn new() -> Self {
        Self { world_model: None }
    }

    /// Attach a PSDG handle for browser state persistence.
    ///
    /// When set, each `get_state()` call persists the URL and page title
    /// to WorldModelStore as fire-and-forget semantic facts.
    pub fn with_world_model(mut self, psdg: crate::agent::psdg::PsdgHandle) -> Self {
        self.world_model = Some(psdg);
        self
    }

    /// Check if CDP-based browser automation is available.
    /// Checks both binary existence AND whether the CDP port is open.
    pub async fn is_available() -> bool {
        let chrome_paths = [
            "/usr/bin/google-chrome",
            "/usr/bin/chromium-browser",
            "/usr/bin/chromium",
        ];
        if !chrome_paths
            .iter()
            .any(|p| std::path::Path::new(p).exists())
        {
            return false;
        }
        let check = tokio::time::timeout(
            tokio::time::Duration::from_millis(200),
            tokio::net::TcpStream::connect("127.0.0.1:9222"),
        )
        .await;
        matches!(check, Ok(Ok(_)))
    }

    /// Scan /proc for any Chrome process running with CDP enabled.
    fn find_cdp_chrome_pid() -> Option<u32> {
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let pid_str = entry.file_name();
                let pid_str = pid_str.to_string_lossy();
                if pid_str.parse::<u32>().is_err() {
                    continue;
                }
                let cmdline_path = entry.path().join("cmdline");
                if let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) {
                    let args = cmdline.replace('\0', " ");
                    let is_chrome = args.contains("/chrome")
                        || args.contains("google-chrome")
                        || args.contains("chromium");
                    let has_cdp = args.contains("--remote-debugging-port");
                    let is_main = !args.contains("--type=");
                    if is_chrome && has_cdp && is_main {
                        if let Ok(pid) = pid_str.parse::<u32>() {
                            return Some(pid);
                        }
                    }
                }
            }
        }
        None
    }

    /// Launch Chrome with remote debugging enabled.
    /// Uses a kria-specific --user-data-dir to avoid ProcessSingleton conflicts
    /// with any existing Chrome instance the user has open.
    pub async fn launch_with_debugging() -> Result<u32, String> {
        // Reuse running Chrome instance if CDP port 9222 is already open
        if Self::is_available().await {
            let existing_pid = MANAGED_BROWSER_PID.load(Ordering::SeqCst);
            if existing_pid > 0 && std::path::Path::new(&format!("/proc/{}", existing_pid)).exists()
            {
                tracing::info!(target: "browser_cognition", pid = existing_pid, "Reusing existing managed Chrome instance");
                return Ok(existing_pid);
            }
            if let Some(pid) = Self::find_cdp_chrome_pid() {
                MANAGED_BROWSER_PID.store(pid, Ordering::SeqCst);
                tracing::info!(target: "browser_cognition", pid = pid, "Found running Chrome with CDP, reusing");
                return Ok(pid);
            }
        }
        // Resolve the Chrome binary at runtime. `which` is the most reliable
        // method — it respects $PATH and alternatives symlinks.
        let chrome_bin = tokio::process::Command::new("which")
            .args([
                "google-chrome",
                "chromium-browser",
                "chromium",
                "google-chrome-stable",
            ])
            .output()
            .await
            .ok()
            .and_then(|o| {
                let stdout = String::from_utf8_lossy(&o.stdout);
                stdout.lines().next().map(|s| s.trim().to_string())
            })
            .filter(|s| !s.is_empty());

        // Fall back to known hard-coded paths if `which` fails
        let chrome_bin = match chrome_bin {
            Some(b) => b,
            None => {
                let paths = [
                    "/opt/google/chrome/google-chrome",
                    "/usr/bin/google-chrome",
                    "/usr/bin/chromium-browser",
                    "/usr/bin/chromium",
                ];
                paths
                    .iter()
                    .find(|p| std::path::Path::new(p).exists())
                    .map(|s| s.to_string())
                    .ok_or_else(|| "Chrome/Chromium not installed".to_string())?
            }
        };

        // Use a kria-owned profile directory so we can launch alongside the user's
        // Chrome without hitting ProcessSingleton conflicts.
        let profile_dir = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("kria-cdp-profile");
        let _ = std::fs::create_dir_all(&profile_dir);

        // Delete stale ProcessSingleton locks/cookies to prevent Chrome from refusing to start
        // if a previous instance was killed abruptly.
        let _ = std::fs::remove_file(profile_dir.join("SingletonLock"));
        let _ = std::fs::remove_file(profile_dir.join("SingletonSocket"));
        let _ = std::fs::remove_file(profile_dir.join("SingletonCookie"));

        // kria-tmp: user-writable temp dir for Chrome's ProcessSingleton socket.
        // On hardened systems /tmp may not be writable (drwxr-xr-x root:root),
        // causing Chrome to fail with "Failed to create socket directory" (EACCES).
        // Setting TMPDIR to a user-owned path resolves this.
        let kria_tmp = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/var/tmp"))
            .join("kria-tmp");
        let _ = std::fs::create_dir_all(&kria_tmp);

        // Pass through the Wayland/X11 session env vars so Chrome can display.
        let xdg_runtime = std::env::var("XDG_RUNTIME_DIR")
            .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
        let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
        let display = std::env::var("DISPLAY").unwrap_or_default();

        let child = tokio::process::Command::new(&chrome_bin)
            .args([
                "--remote-debugging-port=9222",
                &format!("--user-data-dir={}", profile_dir.display()),
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--disable-sync",
                "--disable-default-apps",
                "--metrics-recording-only",
                "--safebrowsing-disable-auto-update",
                "about:blank",
            ])
            .env("TMPDIR", &kria_tmp)
            .env("XDG_RUNTIME_DIR", &xdg_runtime)
            .env("WAYLAND_DISPLAY", &wayland_display)
            .env("DISPLAY", &display)
            .spawn()
            .map_err(|e| format!("Failed to launch Chrome ({}): {}", chrome_bin, e))?;

        let pid = child.id().unwrap_or(0);
        MANAGED_BROWSER_PID.store(pid, Ordering::SeqCst);

        tracing::info!(
            target: "browser_cognition",
            pid = pid,
            binary = %chrome_bin,
            profile = %profile_dir.display(),
            "Chrome launched with CDP on port 9222"
        );

        // Poll until CDP port is ready (up to 10s, 200ms intervals)
        for _ in 0..50 {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let check = tokio::time::timeout(
                tokio::time::Duration::from_millis(100),
                tokio::net::TcpStream::connect("127.0.0.1:9222"),
            )
            .await;
            if matches!(check, Ok(Ok(_))) {
                tracing::info!(
                    target: "browser_cognition",
                    pid = pid,
                    "CDP port 9222 ready"
                );
                return Ok(pid);
            }
        }

        Err(format!(
            "Chrome launched (pid={}) but CDP port 9222 not responding after 10s",
            pid
        ))
    }

    /// Retrieve the PID of the managed browser session, if any.
    pub fn get_managed_pid() -> Option<u32> {
        let pid = MANAGED_BROWSER_PID.load(Ordering::SeqCst);
        if pid > 0 {
            Some(pid)
        } else {
            None
        }
    }

    /// Get the active page WebSocket debugger URL, adhering to target stickiness.
    async fn get_page_ws_url(&self) -> Result<String, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|e| e.to_string())?;

        let resp = client
            .get("http://127.0.0.1:9222/json")
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let tabs: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;

        let target_lock = MANAGED_TARGET_ID.read().unwrap();
        let sticky_target = target_lock.clone();
        drop(target_lock);

        let mut page_tab = None;

        // 1. If we have a sticky target, try to find it first.
        if let Some(target_id) = &sticky_target {
            page_tab = tabs.iter().find(|t| t["id"].as_str() == Some(target_id));
        }

        // 2. If no sticky target or it was closed, pick the first available page tab.
        if page_tab.is_none() {
            page_tab = tabs
                .iter()
                .find(|t| t["type"].as_str() == Some("page"))
                .or_else(|| tabs.first());

            // Save the new target as sticky to prevent chaos stale-tab race conditions.
            if let Some(tab) = page_tab {
                if let Some(id) = tab["id"].as_str() {
                    let mut lock = MANAGED_TARGET_ID.write().unwrap();
                    *lock = Some(id.to_string());
                    tracing::info!(target: "browser_cognition", target_id = id, "Acquired new sticky browser tab target");
                }
            }
        }

        if let Some(tab) = page_tab {
            if let Some(ws_url) = tab["webSocketDebuggerUrl"].as_str() {
                return Ok(ws_url.to_string());
            }
        }

        Err("No page tab found with webSocketDebuggerUrl".into())
    }

    /// Execute a CDP command via WebSocket.
    async fn execute_cdp_command(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let ws_url = self.get_page_ws_url().await?;

        let (ws_stream, _) = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            tokio_tungstenite::connect_async(&ws_url),
        )
        .await
        .map_err(|_| "WebSocket connection timeout")?
        .map_err(|e| format!("WebSocket connection failed: {}", e))?;

        let (mut write, mut read) = ws_stream.split();

        let req = serde_json::json!({
            "id": 1,
            "method": method,
            "params": params
        });

        write
            .send(Message::Text(req.to_string().into()))
            .await
            .map_err(|e| format!("Failed to send CDP message: {}", e))?;

        while let Some(msg) = tokio::time::timeout(std::time::Duration::from_secs(5), read.next())
            .await
            .ok()
            .flatten()
        {
            let msg = msg.map_err(|e| format!("WebSocket read error: {}", e))?;
            if let Message::Text(text) = msg {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                    if parsed["id"].as_i64() == Some(1) {
                        return Ok(parsed);
                    }
                }
            }
        }

        Err("No valid response received from CDP".into())
    }

    /// Get the current browser state via CDP.
    pub async fn get_state(&self) -> BrowserState {
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
        {
            Ok(c) => c,
            Err(_) => return BrowserState::unknown(),
        };

        let resp = match client.get("http://127.0.0.1:9222/json").send().await {
            Ok(r) => r,
            Err(_) => return BrowserState::unknown(),
        };

        let tabs: Vec<serde_json::Value> = match resp.json().await {
            Ok(t) => t,
            Err(_) => return BrowserState::unknown(),
        };

        let target_lock = MANAGED_TARGET_ID.read().unwrap();
        let sticky_target = target_lock.clone();
        drop(target_lock);

        let mut page_tab = None;
        if let Some(target_id) = &sticky_target {
            page_tab = tabs.iter().find(|t| t["id"].as_str() == Some(target_id));
        }
        if page_tab.is_none() {
            page_tab = tabs
                .iter()
                .find(|t| t["type"].as_str() == Some("page"))
                .or_else(|| tabs.first());
        }

        if let Some(tab) = page_tab {
            let url = tab["url"].as_str().unwrap_or("").to_string();
            let title = tab["title"].as_str().unwrap_or("").to_string();

            let state = BrowserState {
                url: url.chars().take(500).collect(),
                title: title.chars().take(200).collect(),
                loading: false,
                active_tab: 0,
                tab_count: tabs.len(),
                dialog_visible: false,
                dialog_message: None,
            };

            // ── PSDG: persist browser navigation state (fire-and-forget) ────
            if !state.url.is_empty() {
                if let Some(ref psdg) = self.world_model {
                    psdg.record_browser_navigation(&state.url, &state.title);
                }
            }

            state
        } else {
            BrowserState::unknown()
        }
    }

    /// Navigate to a URL in the current browser tab.
    pub async fn navigate(&self, url: &str) -> BrowserResult {
        match self
            .execute_cdp_command("Page.navigate", serde_json::json!({ "url": url }))
            .await
        {
            Ok(_) => BrowserResult::ok(format!("Navigated to {}", url)),
            Err(e) => BrowserResult::err(format!("Navigation failed: {}", e)),
        }
    }

    /// Wait for the DOM to stabilize (useful for SPA transitions).
    /// Injects a MutationObserver and resolves when no mutations occur for 1000ms.
    /// Emits timeout safely after 5 seconds to prevent unbounded hangs.
    pub async fn wait_for_spa_transition(&self) -> BrowserResult {
        let expr = r#"
            new Promise((resolve) => {
                let idleTimeout;
                let failTimeout;
                const observer = new MutationObserver(() => {
                    clearTimeout(idleTimeout);
                    idleTimeout = setTimeout(() => {
                        observer.disconnect();
                        clearTimeout(failTimeout);
                        resolve("DOM stable");
                    }, 1000);
                });
                observer.observe(document.body || document.documentElement, { childList: true, subtree: true, attributes: true });
                idleTimeout = setTimeout(() => {
                    observer.disconnect();
                    clearTimeout(failTimeout);
                    resolve("DOM stable (no initial mutations)");
                }, 1000);
                failTimeout = setTimeout(() => {
                    observer.disconnect();
                    clearTimeout(idleTimeout);
                    resolve("DOM stable (timeout reached)");
                }, 5000);
            })
        "#;

        // Use a longer CDP timeout since the script itself can take up to 5s to resolve.
        let ws_url = match self.get_page_ws_url().await {
            Ok(url) => url,
            Err(e) => return BrowserResult::err(format!("SPA wait failed: {}", e)),
        };

        let (ws_stream, _) = match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            tokio_tungstenite::connect_async(&ws_url),
        )
        .await
        {
            Ok(Ok(s)) => s,
            _ => return BrowserResult::err("WebSocket connection failed for SPA wait"),
        };

        let (mut write, mut read) = ws_stream.split();
        let req = serde_json::json!({
            "id": 1,
            "method": "Runtime.evaluate",
            "params": {
                "expression": expr,
                "awaitPromise": true
            }
        });

        if write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                req.to_string().into(),
            ))
            .await
            .is_err()
        {
            return BrowserResult::err("Failed to send SPA wait command");
        }

        while let Some(msg) = tokio::time::timeout(std::time::Duration::from_secs(8), read.next())
            .await
            .ok()
            .flatten()
        {
            if let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = msg {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                    if parsed["id"].as_i64() == Some(1) {
                        return BrowserResult::ok("SPA transition completed (DOM stable)");
                    }
                }
            }
        }

        BrowserResult::err("SPA wait failed: CDP timeout")
    }

    /// Get the text content of the current page.
    pub async fn get_page_text(&self) -> BrowserResult {
        let expr = "document.body ? document.body.innerText.substring(0, 50000) : ''";
        match self
            .execute_cdp_command(
                "Runtime.evaluate",
                serde_json::json!({ "expression": expr }),
            )
            .await
        {
            Ok(res) => {
                let text = res["result"]["result"]["value"].as_str().unwrap_or("");
                BrowserResult::ok_with_data(
                    format!("Got page text ({} chars)", text.len()),
                    serde_json::json!({"text": text}),
                )
            }
            Err(e) => BrowserResult::err(format!("CDP error: {}", e)),
        }
    }

    /// Read safe focus metadata from the active Chrome/Chromium page via CDP.
    ///
    /// This intentionally returns only element metadata: role/label/bounds/editable
    /// state and a safe page title/origin summary. It does not return page text.
    pub async fn get_chrome_focus_snapshot(&self) -> BrowserResult {
        let expr = browser_focus_expression("chrome");
        match self
            .execute_cdp_command(
                "Runtime.evaluate",
                serde_json::json!({
                    "expression": expr,
                    "returnByValue": true,
                    "awaitPromise": false
                }),
            )
            .await
        {
            Ok(res) => {
                let value = res["result"]["result"]["value"].clone();
                if value.get("status").and_then(serde_json::Value::as_str) == Some("ok") {
                    BrowserResult::ok_with_data("Chrome CDP active element focus metadata", value)
                } else {
                    BrowserResult::err(
                        value
                            .get("reason")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("Chrome CDP did not expose an active element")
                            .to_string(),
                    )
                }
            }
            Err(error) => BrowserResult::err(format!("Chrome CDP focus probe failed: {error}")),
        }
    }

    /// Read safe focus metadata from Firefox through WebDriver BiDi.
    ///
    /// Firefox must be launched with `--remote-debugging-port=<port>`. The probe
    /// creates a temporary BiDi session, gets the first top-level browsing
    /// context, and evaluates the same metadata-only active element script.
    pub async fn get_firefox_bidi_focus_snapshot(port: u16) -> BrowserResult {
        let ws_url = format!("ws://127.0.0.1:{port}/session");
        let connected = tokio::time::timeout(
            std::time::Duration::from_millis(220),
            tokio_tungstenite::connect_async(&ws_url),
        )
        .await;
        let (ws_stream, _) = match connected {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) => {
                return BrowserResult::err(format!("Firefox BiDi unavailable: {error}"));
            }
            Err(_) => return BrowserResult::err("Firefox BiDi unavailable: connection timeout"),
        };
        let (mut write, mut read) = ws_stream.split();

        async fn send_bidi(
            write: &mut futures::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
                Message,
            >,
            read: &mut futures::stream::SplitStream<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
            >,
            id: u64,
            method: &str,
            params: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            let req = serde_json::json!({
                "id": id,
                "method": method,
                "params": params,
            });
            write
                .send(Message::Text(req.to_string().into()))
                .await
                .map_err(|error| format!("Firefox BiDi send failed: {error}"))?;
            while let Some(msg) =
                tokio::time::timeout(std::time::Duration::from_millis(220), read.next())
                    .await
                    .map_err(|_| "Firefox BiDi response timeout".to_string())?
            {
                let msg = msg.map_err(|error| format!("Firefox BiDi read failed: {error}"))?;
                if let Message::Text(text) = msg {
                    let parsed: serde_json::Value =
                        serde_json::from_str(&text).map_err(|error| error.to_string())?;
                    if parsed.get("id").and_then(serde_json::Value::as_u64) == Some(id) {
                        return Ok(parsed);
                    }
                }
            }
            Err("Firefox BiDi connection closed".into())
        }

        if let Err(error) = send_bidi(
            &mut write,
            &mut read,
            1,
            "session.new",
            serde_json::json!({"capabilities": {}}),
        )
        .await
        {
            return BrowserResult::err(error);
        }

        let tree = match send_bidi(
            &mut write,
            &mut read,
            2,
            "browsingContext.getTree",
            serde_json::json!({}),
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                let _ = send_bidi(
                    &mut write,
                    &mut read,
                    99,
                    "session.end",
                    serde_json::json!({}),
                )
                .await;
                return BrowserResult::err(error);
            }
        };
        let context = tree["result"]["contexts"]
            .as_array()
            .and_then(|contexts| contexts.first())
            .and_then(|context| context.get("context"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let Some(context) = context else {
            return BrowserResult::err("Firefox BiDi did not expose a browsing context");
        };

        let evaluated = match send_bidi(
            &mut write,
            &mut read,
            3,
            "script.evaluate",
            serde_json::json!({
                "expression": browser_focus_expression("firefox"),
                "target": { "context": context },
                "awaitPromise": false,
                "resultOwnership": "none",
                "serializationOptions": { "maxObjectDepth": 4 }
            }),
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                let _ = send_bidi(
                    &mut write,
                    &mut read,
                    99,
                    "session.end",
                    serde_json::json!({}),
                )
                .await;
                return BrowserResult::err(error);
            }
        };

        let value = webdriver_bidi_local_value_to_json(&evaluated["result"]["result"]);
        let _ = send_bidi(
            &mut write,
            &mut read,
            4,
            "session.end",
            serde_json::json!({}),
        )
        .await;
        if value.get("status").and_then(serde_json::Value::as_str) == Some("ok") {
            BrowserResult::ok_with_data("Firefox BiDi active element focus metadata", value)
        } else {
            BrowserResult::err(
                value
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Firefox BiDi did not expose an active element")
                    .to_string(),
            )
        }
    }

    /// Click an element on the page by CSS selector or text content.
    pub async fn click_element(&self, selector: &str) -> BrowserResult {
        let sel_json = serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".to_string());
        let sel_lower_json =
            serde_json::to_string(&selector.to_lowercase()).unwrap_or_else(|_| "\"\"".to_string());

        let expr = format!(
            r#"(function() {{
                var sel = {};
                var selLower = {};
                var el = null;
                try {{ el = document.querySelector(sel); }} catch(e) {{}}
                if (!el) {{
                    var all = document.querySelectorAll('button, a, input[type=submit], input[type=button]');
                    for (var i = 0; i < all.length; i++) {{
                        if (all[i].textContent.trim().toLowerCase().includes(selLower)) {{
                            el = all[i]; break;
                        }}
                    }}
                }}
                if (el) {{ el.click(); return 'clicked: ' + (el.textContent || el.value || el.id || 'element'); }}
                return 'not found';
            }})()"#,
            sel_json, sel_lower_json
        );

        match self
            .execute_cdp_command(
                "Runtime.evaluate",
                serde_json::json!({ "expression": expr }),
            )
            .await
        {
            Ok(res) => {
                let val = res["result"]["result"]["value"].as_str().unwrap_or("");
                if !val.is_empty() && val != "not found" {
                    BrowserResult::ok(val)
                } else {
                    BrowserResult::err(format!("Element not found: {}", selector))
                }
            }
            Err(e) => BrowserResult::err(format!("Click failed: {}", e)),
        }
    }

    /// Fill a form field by label or placeholder text.
    pub async fn fill_field(&self, label: &str, value: &str) -> BrowserResult {
        let label_lower_json =
            serde_json::to_string(&label.to_lowercase()).unwrap_or_else(|_| "\"\"".to_string());
        let val_json = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string());

        let expr = format!(
            r#"(function() {{
                var labelStr = {};
                var val = {};
                var inputs = document.querySelectorAll('input, textarea');
                for (var i = 0; i < inputs.length; i++) {{
                    var inp = inputs[i];
                    var lbl = document.querySelector('label[for="' + inp.id + '"]');
                    var lblText = lbl ? lbl.textContent.trim().toLowerCase() : '';
                    var ph = (inp.placeholder || '').toLowerCase();
                    var nm = (inp.name || '').toLowerCase();
                    if (lblText.includes(labelStr) || ph.includes(labelStr) || nm.includes(labelStr)) {{
                        inp.value = val;
                        inp.dispatchEvent(new Event('input', {{bubbles: true}}));
                        inp.dispatchEvent(new Event('change', {{bubbles: true}}));
                        return 'filled: ' + (inp.id || inp.name || inp.placeholder || 'field');
                    }}
                }}
                return 'not found';
            }})()"#,
            label_lower_json, val_json
        );

        match self
            .execute_cdp_command(
                "Runtime.evaluate",
                serde_json::json!({ "expression": expr }),
            )
            .await
        {
            Ok(res) => {
                let val = res["result"]["result"]["value"].as_str().unwrap_or("");
                if !val.is_empty() && val != "not found" {
                    BrowserResult::ok(val)
                } else {
                    BrowserResult::err(format!("Field not found: {}", label))
                }
            }
            Err(e) => BrowserResult::err(format!("Fill failed: {}", e)),
        }
    }
}

impl Default for BrowserCognitionEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn browser_focus_expression(browser: &str) -> String {
    let browser_json = serde_json::to_string(browser).unwrap_or_else(|_| "\"browser\"".into());
    format!(
        r#"(function() {{
            const browser = {browser_json};
            const el = document.activeElement;
            const redact = (value, limit = 160) => {{
                if (value === null || value === undefined) return null;
                const text = String(value)
                    .replace(/[\u0000-\u001f\u007f]/g, " ")
                    .replace(/((?:api[_-]?key|token|password|passwd|secret|credential|authorization|bearer)\s*[:=]\s*)[^\s,;]+/gi, "$1[REDACTED]")
                    .trim();
                return text ? text.slice(0, limit) : null;
            }};
            const hash = (value) => {{
                const text = String(value || "");
                let h = 2166136261;
                for (let i = 0; i < text.length; i++) {{
                    h ^= text.charCodeAt(i);
                    h = Math.imul(h, 16777619);
                }}
                return "h" + (h >>> 0).toString(16);
            }};
            if (!el) {{
                return {{ status: "unavailable", browser, reason: "document.activeElement is unavailable" }};
            }}
            const tag = (el.tagName || "").toLowerCase();
            const type = (el.getAttribute("type") || "").toLowerCase();
            const explicitRole = el.getAttribute("role");
            const role = explicitRole || (
                tag === "textarea" ? "textbox" :
                tag === "select" ? "combobox" :
                tag === "button" ? "button" :
                tag === "a" ? "link" :
                tag === "input" && type === "search" ? "searchbox" :
                tag === "input" && ["button", "submit", "reset"].includes(type) ? "button" :
                tag === "input" ? "textbox" :
                el.isContentEditable ? "textbox" :
                tag || "unknown"
            );
            const disabled = Boolean(el.disabled || el.getAttribute("aria-disabled") === "true");
            const readonly = Boolean(el.readOnly || el.getAttribute("aria-readonly") === "true");
            const editable = !disabled && !readonly && (
                el.isContentEditable ||
                tag === "textarea" ||
                tag === "select" ||
                (tag === "input" && !["button", "submit", "reset", "checkbox", "radio", "file", "hidden"].includes(type))
            );
            const rect = el.getBoundingClientRect ? el.getBoundingClientRect() : null;
            const label = redact(
                el.getAttribute("aria-label") ||
                el.getAttribute("placeholder") ||
                el.getAttribute("name") ||
                el.getAttribute("title") ||
                el.getAttribute("value") ||
                el.id ||
                role,
                140
            );
            return {{
                status: "ok",
                browser,
                tag,
                role,
                label,
                id_hash: hash(el.id),
                class_hash: hash(el.className),
                input_type: redact(type, 40),
                editable,
                disabled,
                readonly,
                is_content_editable: Boolean(el.isContentEditable),
                bounds: rect ? {{
                    x: Math.round(rect.left + window.screenX),
                    y: Math.round(rect.top + window.screenY),
                    width: Math.round(rect.width),
                    height: Math.round(rect.height)
                }} : null,
                page_title: redact(document.title, 160),
                url_origin: redact(location.origin, 160),
                document_has_focus: Boolean(document.hasFocus && document.hasFocus()),
                observed_at_ms: Date.now()
            }};
        }})()"#
    )
}

fn webdriver_bidi_local_value_to_json(value: &serde_json::Value) -> serde_json::Value {
    let Some(kind) = value.get("type").and_then(serde_json::Value::as_str) else {
        return value.clone();
    };
    match kind {
        "object" => {
            let mut map = serde_json::Map::new();
            if let Some(entries) = value.get("value").and_then(serde_json::Value::as_array) {
                for entry in entries {
                    let Some(pair) = entry.as_array() else {
                        continue;
                    };
                    if pair.len() != 2 {
                        continue;
                    }
                    let key = pair[0]
                        .get("value")
                        .and_then(serde_json::Value::as_str)
                        .or_else(|| pair[0].as_str());
                    if let Some(key) = key {
                        map.insert(
                            key.to_string(),
                            webdriver_bidi_local_value_to_json(&pair[1]),
                        );
                    }
                }
            }
            serde_json::Value::Object(map)
        }
        "array" => serde_json::Value::Array(
            value
                .get("value")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(webdriver_bidi_local_value_to_json)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        ),
        "string" => value
            .get("value")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "number" | "boolean" => value
            .get("value")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "null" | "undefined" => serde_json::Value::Null,
        _ => value.get("value").cloned().unwrap_or_else(|| value.clone()),
    }
}
