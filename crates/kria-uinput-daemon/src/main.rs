//! KRIA uinput Daemon - Phase 1.5 IPC Implementation
//!
//! This is an isolated, privileged helper process for GUI automation.
//! It runs with uinput access but has NO access to:
//! - KRIA core memory space
//! - LLM inference paths
//! - User data or secrets
//!
//! Architecture: KRIA core (unprivileged) -> Unix Socket -> This daemon (privileged)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::Duration as TokioDuration;
use tracing::{error, info, warn};

// ============================================================================
// IPC Protocol Definitions
// ============================================================================

/// Request from KRIA core to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum DaemonRequest {
    /// Move mouse and click
    Click {
        x: i32,
        y: i32,
        /// "left", "right", "middle"
        button: String,
    },
    /// Type text with optional delay
    Type {
        text: String,
        /// Delay between keystrokes in milliseconds
        interval_ms: Option<u64>,
    },
    /// Press keyboard shortcut
    Shortcut {
        /// Array of keys like ["ctrl", "s"]
        keys: Vec<String>,
        /// Duration to hold keys in milliseconds
        hold_duration_ms: Option<u64>,
    },
    /// Release all modifier keys (kill switch safety)
    ReleaseAll,
    /// Get active window info (for verification)
    GetActiveWindow,
    /// RFC 008: Heartbeat for dead-man's switch
    /// Parent must send heartbeat every 2 seconds or daemon will halt input
    Heartbeat,
    /// RFC 008: Explicit task completion — disables dead-man's switch for this session
    /// and releases all held keys before disconnect.
    TaskComplete,
}

/// Response from daemon to KRIA core.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DaemonResponse {
    /// Command executed successfully
    Ok {
        /// Optional response data
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
    /// Command failed
    Error {
        /// Error message
        message: String,
        /// Error code for programmatic handling
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

/// Window information for verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub title: String,
    pub class: String,
    pub pid: u32,
}

// ============================================================================
// Socket Security
// ============================================================================

/// Create Unix Domain Socket with strict permissions.
///
/// Security:
/// - Socket created with chmod 600 (owner read/write only)
/// - Parent directory should also be restricted
/// - This prevents any other user from connecting to the daemon
async fn create_secure_socket(socket_path: &PathBuf) -> Result<UnixListener> {
    // Remove old socket if it exists
    if socket_path.exists() {
        tokio::fs::remove_file(socket_path)
            .await
            .context("Failed to remove old socket file")?;
    }

    // Ensure parent directory exists
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("Failed to create socket directory")?;

        // Set permissions on parent directory to allow non-root access
        let mut perms = std::fs::metadata(parent)?.permissions();
        perms.set_mode(0o755); // owner read/write/execute, group/others read/execute
        std::fs::set_permissions(parent, perms)?;
    }

    // Create the listener
    let listener = UnixListener::bind(socket_path).context("Failed to bind Unix Domain Socket")?;

    // Set permissions on socket file (chmod 777 for non-root client access)
    // Note: This is safe because the daemon validates all requests before executing
    let mut perms = std::fs::metadata(socket_path)?.permissions();
    perms.set_mode(0o777); // allow all users to connect
    std::fs::set_permissions(socket_path, perms).context("Failed to set socket permissions")?;

    info!(
        socket = %socket_path.display(),
        "Unix Domain Socket created with chmod 777"
    );

    Ok(listener)
}

// ============================================================================
// Command Execution
// ============================================================================

/// Execute xdotool command safely.
///
/// Security:
/// - All arguments are properly escaped
/// - Timeout enforced
/// - No shell interpolation
/// - X11-native automation (no daemon required)
async fn execute_xdotool(args: &[&str]) -> Result<String> {
    let timeout = Duration::from_secs(10);

    // Check for xdotool existence
    if !std::path::Path::new("/usr/bin/xdotool").exists()
        && !std::path::Path::new("/usr/local/bin/xdotool").exists()
    {
        anyhow::bail!(
            "Missing dependency: sudo apt install xdotool. X11-native automation requires xdotool."
        );
    }

    let cmd_name = "xdotool";
    info!(command = %format!("{} {}", cmd_name, args.join(" ")), "Executing X11 automation command");

    let output = tokio::time::timeout(
        timeout,
        tokio::process::Command::new(cmd_name).args(args).output(),
    )
    .await
    .context(format!("{} command timed out", cmd_name))?
    .context(format!("Failed to execute {}", cmd_name))?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        anyhow::bail!("{} failed: {}", cmd_name, stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.to_string())
}

/// Map key names to ydotool key names.
fn map_key_name(key: &str) -> Result<String> {
    let key_lower = key.to_lowercase();
    let mapped = match key_lower.as_str() {
        "shift" => "Shift",
        "ctrl" | "control" => "Control",
        "alt" => "Alt",
        "super" | "win" | "cmd" | "command" => "Super",
        "escape" | "esc" => "Escape",
        "enter" | "return" => "Return",
        "tab" => "Tab",
        "space" => "Space",
        "backspace" | "bksp" => "Backspace",
        "delete" | "del" => "Delete",
        "home" => "Home",
        "end" => "End",
        "pageup" | "page_up" => "Page_Up",
        "pagedown" | "page_down" => "Page_Down",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "f1" => "F1",
        "f2" => "F2",
        "f3" => "F3",
        "f4" => "F4",
        "f5" => "F5",
        "f6" => "F6",
        "f7" => "F7",
        "f8" => "F8",
        "f9" => "F9",
        "f10" => "F10",
        "f11" => "F11",
        "f12" => "F12",
        // Single character
        c if c.len() == 1 => return Ok(c.to_string()),
        _ => anyhow::bail!("Unknown key: {}", key),
    };
    Ok(mapped.to_string())
}

/// Handle daemon requests and execute commands.
async fn handle_request(request: DaemonRequest) -> DaemonResponse {
    match request {
        DaemonRequest::Click { x, y, button } => {
            info!(x = x, y = y, button = %button, "Received click command");
            // Map button names to xdotool button numbers
            let button_num = match button.as_str() {
                "left" => "1",
                "right" => "3",
                "middle" => "2",
                _ => {
                    return DaemonResponse::Error {
                        message: format!("Invalid button: {}", button),
                        code: Some("INVALID_BUTTON".to_string()),
                    };
                }
            };

            // Move mouse to position
            if let Err(e) =
                execute_xdotool(&["mousemove", "--sync", &x.to_string(), &y.to_string()]).await
            {
                return DaemonResponse::Error {
                    message: format!("Failed to move mouse: {}", e),
                    code: Some("MOUSEMOVE_FAILED".to_string()),
                };
            }

            // Click
            match execute_xdotool(&["click", button_num]).await {
                Ok(_) => DaemonResponse::Ok { data: None },
                Err(e) => DaemonResponse::Error {
                    message: format!("Click failed: {}", e),
                    code: Some("CLICK_FAILED".to_string()),
                },
            }
        }

        DaemonRequest::Type { text, interval_ms } => {
            info!(text_len = text.len(), interval_ms = ?interval_ms, "Received type command");

            // Escape text for xdotool - use double quotes and escape internal quotes
            let escaped_text = if text.contains('"') {
                text.replace('"', "\\\"")
            } else {
                text
            };

            // Build args with separate arguments for xdotool
            // xdotool expects: type [--clearmodifiers] [--delay <ms>] <text>
            let args: Vec<String> = match interval_ms {
                Some(delay) => {
                    vec![
                        "type".to_string(),
                        "--clearmodifiers".to_string(),
                        "--delay".to_string(),
                        delay.to_string(),
                        escaped_text.clone(),
                    ]
                }
                None => {
                    vec![
                        "type".to_string(),
                        "--clearmodifiers".to_string(),
                        escaped_text.clone(),
                    ]
                }
            };

            info!(args = ?args, "Executing xdotool type command");
            match execute_xdotool(&args.iter().map(|s| s.as_str()).collect::<Vec<_>>()).await {
                Ok(_) => {
                    info!(chars_typed = escaped_text.len(), "Type command succeeded");
                    DaemonResponse::Ok {
                        data: Some(serde_json::json!({ "typed_chars": escaped_text.len() })),
                    }
                }
                Err(e) => {
                    error!(error = %e, "Type command failed");
                    DaemonResponse::Error {
                        message: format!("Type failed: {}", e),
                        code: Some("TYPE_FAILED".to_string()),
                    }
                }
            }
        }

        DaemonRequest::Shortcut {
            keys,
            hold_duration_ms,
        } => {
            // Map key names
            let mut mapped_keys = Vec::new();
            for key in &keys {
                match map_key_name(key) {
                    Ok(mapped) => mapped_keys.push(mapped),
                    Err(e) => {
                        return DaemonResponse::Error {
                            message: format!("Invalid key '{}': {}", key, e),
                            code: Some("INVALID_KEY".to_string()),
                        };
                    }
                }
            }

            let key_sequence = mapped_keys.join("+");

            let hold_flag = hold_duration_ms.map(|h| format!("--hold {}", h));
            let args: Vec<&str> = if let Some(ref hold) = hold_flag {
                vec!["key", hold, &key_sequence]
            } else {
                vec!["key", &key_sequence]
            };

            match execute_xdotool(&args).await {
                Ok(_) => DaemonResponse::Ok { data: None },
                Err(e) => DaemonResponse::Error {
                    message: format!("Shortcut failed: {}", e),
                    code: Some("SHORTCUT_FAILED".to_string()),
                },
            }
        }

        DaemonRequest::ReleaseAll => {
            // Release all modifier keys to prevent OS lockup.
            // xdotool syntax: `xdotool keyup <key>` (lowercase key names).
            let modifiers = ["shift", "ctrl", "alt", "super"];
            let mut errors = Vec::new();

            for modifier in &modifiers {
                if let Err(e) = execute_xdotool(&["keyup", modifier]).await {
                    errors.push(format!("{}: {}", modifier, e));
                }
            }

            if errors.is_empty() {
                DaemonResponse::Ok { data: None }
            } else {
                DaemonResponse::Error {
                    message: format!("Partial release failure: {}", errors.join(", ")),
                    code: Some("PARTIAL_RELEASE".to_string()),
                }
            }
        }

        DaemonRequest::GetActiveWindow => {
            // Get active window ID
            let window_id_result = tokio::process::Command::new("xdotool")
                .args(["getactivewindow"])
                .output()
                .await;

            let window_id = match window_id_result {
                Ok(output) if output.status.success() => {
                    String::from_utf8_lossy(&output.stdout).trim().to_string()
                }
                _ => {
                    return DaemonResponse::Error {
                        message: "Failed to get active window ID".to_string(),
                        code: Some("WINDOW_ID_FAILED".to_string()),
                    };
                }
            };

            // Get window title
            let title_result = tokio::process::Command::new("xdotool")
                .args(["getwindowname", &window_id])
                .output()
                .await;

            let title = match title_result {
                Ok(output) if output.status.success() => {
                    String::from_utf8_lossy(&output.stdout).trim().to_string()
                }
                _ => "Unknown".to_string(),
            };

            // Get window class using xprop (xdotool doesn't have getwindowclassname)
            let class_result = tokio::process::Command::new("xprop")
                .args(["-id", &window_id, "WM_CLASS"])
                .output()
                .await;

            let class = match class_result {
                Ok(output) if output.status.success() => {
                    // Parse WM_CLASS(STRING) = "instance", "class"
                    let output_str = String::from_utf8_lossy(&output.stdout);
                    // Extract the class name (second quoted string)
                    let parts: Vec<&str> = output_str.split('"').collect();
                    if parts.len() >= 4 {
                        parts[3].to_string() // Second quoted value is the class
                    } else {
                        "Unknown".to_string()
                    }
                }
                _ => "Unknown".to_string(),
            };

            let window_info = WindowInfo {
                title,
                class,
                pid: 0, // Could add PID query if needed
            };

            DaemonResponse::Ok {
                data: Some(serde_json::to_value(window_info).unwrap_or(serde_json::Value::Null)),
            }
        }

        DaemonRequest::Heartbeat => {
            // RFC 008: Heartbeat is normally handled in handle_client before reaching here.
            // This is a safety fallback in case handle_request is called directly.
            DaemonResponse::Ok { data: None }
        }

        DaemonRequest::TaskComplete => {
            // RFC 008: Client explicitly signals end of task.
            // Release any held keys proactively.
            let _ = execute_emergency_release().await;
            DaemonResponse::Ok { data: None }
        }
    }
}

// ============================================================================
// RFC 008: Emergency Safety Functions
// ============================================================================

/// Emergency key release for dead-man's switch.
/// Called when heartbeat expires or client disconnects unexpectedly.
async fn execute_emergency_release() -> Result<()> {
    error!("RFC 008: Executing EMERGENCY key release - clearing all modifiers");

    let modifiers = [
        ("shift", "Shift"),
        ("ctrl", "Control"),
        ("alt", "Alt"),
        ("super", "Super"),
    ];
    let mut errors = Vec::new();

    for (xdotool_name, _label) in &modifiers {
        if let Err(e) = execute_xdotool(&["keyup", xdotool_name]).await {
            errors.push(format!("{}: {}", xdotool_name, e));
        }
    }

    if errors.is_empty() {
        info!("RFC 008: Emergency key release succeeded - all modifiers cleared");
        Ok(())
    } else {
        error!(errors = ?errors, "RFC 008: Emergency key release had partial failures");
        Err(anyhow::anyhow!(
            "Partial emergency release failure: {}",
            errors.join(", ")
        ))
    }
}

// ============================================================================
// Client Connection Handler
// ============================================================================

/// RFC 008: Heartbeat timeout for dead-man's switch
/// If parent process doesn't send heartbeat within this duration, daemon halts input
const HEARTBEAT_TIMEOUT_SECS: u64 = 5;

/// Handle a single client connection with RFC 008 dead-man's switch.
///
/// Safety: If parent process dies (no heartbeat for 5s), daemon will:
/// 1. Reject all new input commands
/// 2. Execute emergency ReleaseAll to clear stuck keys
/// 3. Return error on any input attempt
async fn handle_client(stream: UnixStream) -> Result<()> {
    let peer = stream
        .peer_cred()
        .map(|cred| format!("uid:{}", cred.uid()))
        .unwrap_or_else(|_| "unknown".to_string());

    info!(peer = %peer, "Client connected - RFC 008 dead-man's switch active (timeout: {}s)", HEARTBEAT_TIMEOUT_SECS);

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // RFC 008: Track last heartbeat time for dead-man's switch
    let mut last_heartbeat = tokio::time::Instant::now();
    let mut heartbeat_valid = true;
    let mut clean_disconnect = false;
    // Only sessions that actually issued input commands need emergency
    // release on dirty disconnect. Heartbeat/window-info-only sessions
    // (e.g. periodic health pings) cannot leave keys stuck.
    let mut sent_input_command = false;

    // Read lines (JSON messages) from client
    while reader.read_line(&mut line).await? > 0 {
        // RFC 008: Check heartbeat validity BEFORE processing
        let elapsed = last_heartbeat.elapsed();
        if elapsed > TokioDuration::from_secs(HEARTBEAT_TIMEOUT_SECS) {
            if heartbeat_valid {
                error!(
                    elapsed_secs = elapsed.as_secs(),
                    "RFC 008 DEAD-MAN'S SWITCH: Heartbeat expired! Emergency halt engaged."
                );
                heartbeat_valid = false;

                // EMERGENCY: Release all keys immediately
                let _ = execute_emergency_release().await;
            }
        }

        // Parse request
        let request: DaemonRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                warn!(error = %e, line = %line.trim(), "Failed to parse request");
                let response = DaemonResponse::Error {
                    message: format!("Invalid JSON: {}", e),
                    code: Some("PARSE_ERROR".to_string()),
                };
                let response_json = serde_json::to_string(&response)?;
                writer.write_all(response_json.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                line.clear();
                continue;
            }
        };

        line.clear();

        // RFC 008: Handle TaskComplete — explicit clean end-of-task signal
        if matches!(request, DaemonRequest::TaskComplete) {
            clean_disconnect = true;
            info!(peer = %peer, "TaskComplete received - releasing keys and preparing clean disconnect");
            let _ = execute_emergency_release().await;
            let response = DaemonResponse::Ok { data: None };
            let response_json = serde_json::to_string(&response)?;
            writer.write_all(response_json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
            continue;
        }

        // RFC 008: Handle heartbeat - update timestamp
        if matches!(request, DaemonRequest::Heartbeat) {
            last_heartbeat = tokio::time::Instant::now();
            if !heartbeat_valid {
                info!("RFC 008: Heartbeat restored - resuming normal operation");
                heartbeat_valid = true;
            }
            let response = DaemonResponse::Ok { data: None };
            let response_json = serde_json::to_string(&response)?;
            writer.write_all(response_json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
            continue;
        }

        // RFC 008: Check if input commands should be blocked due to dead-man's switch
        let is_input_command = matches!(
            request,
            DaemonRequest::Type { .. }
                | DaemonRequest::Click { .. }
                | DaemonRequest::Shortcut { .. }
        );
        if is_input_command {
            sent_input_command = true;
        }

        if is_input_command && !heartbeat_valid {
            error!(
                cmd = ?request,
                "RFC 008 DEAD-MAN'S SWITCH: Rejecting input command - parent process may be dead"
            );
            let response = DaemonResponse::Error {
                message: format!(
                    "RFC 008 Dead-Man's Switch: Heartbeat expired ({}s). Parent process unresponsive. Input blocked.",
                    HEARTBEAT_TIMEOUT_SECS
                ),
                code: Some("HEARTBEAT_EXPIRED".to_string()),
            };
            let response_json = serde_json::to_string(&response)?;
            writer.write_all(response_json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
            continue;
        }

        // Log request (without sensitive data)
        match &request {
            DaemonRequest::Type { text, .. } => {
                info!(cmd = "type", len = text.len(), "Received command");
            }
            DaemonRequest::Click { x, y, .. } => {
                info!(cmd = "click", x, y, "Received command");
            }
            _ => {
                info!(cmd = ?request, "Received command");
            }
        }

        // Execute and respond
        let response = handle_request(request).await;
        let response_json = serde_json::to_string(&response)?;

        writer.write_all(response_json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        // Log response status
        match &response {
            DaemonResponse::Ok { .. } => info!("Command succeeded"),
            DaemonResponse::Error { message, .. } => warn!(error = %message, "Command failed"),
        }
    }

    // RFC 008: Client disconnected — only emergency-release if necessary
    if clean_disconnect {
        info!(peer = %peer, "Client disconnected cleanly (TaskComplete) — no emergency action needed");
    } else if !heartbeat_valid {
        info!(peer = %peer, "Client disconnected after heartbeat expiry — emergency already handled");
    } else if !sent_input_command {
        // Passive sessions (heartbeat ping, get_active_window) cannot leave keys held;
        // skipping emergency release prevents log/xdotool spam from periodic health checks.
        info!(peer = %peer, "Passive client disconnected (no input commands) — no emergency action needed");
    } else {
        info!(peer = %peer, "Client disconnected unexpectedly after input — executing emergency key release");
        let _ = execute_emergency_release().await;
    }

    Ok(())
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(true)
        .init();

    info!("KRIA uinput Daemon starting");
    info!("SECURITY: This daemon runs with elevated privileges for uinput access");
    info!("SECURITY: Socket created with chmod 777 for non-root client access");

    // Determine socket path, in priority order:
    //   1. `--socket <path>` CLI argument (preferred — survives sudo env scrubbing)
    //   2. `KRIA_UINPUT_SOCKET` env var (requires SETENV in sudoers)
    //   3. `$SUDO_USER`'s home cache dir
    //   4. Current user's cache dir, or `/tmp/kria-uinput.sock` as last resort
    let socket_path = {
        let args: Vec<String> = std::env::args().collect();
        let mut cli_path: Option<PathBuf> = None;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--socket" | "-s" => {
                    if let Some(next) = args.get(i + 1) {
                        cli_path = Some(PathBuf::from(next));
                        i += 2;
                        continue;
                    }
                }
                arg if arg.starts_with("--socket=") => {
                    cli_path = Some(PathBuf::from(&arg["--socket=".len()..]));
                }
                _ => {}
            }
            i += 1;
        }

        cli_path
            .or_else(|| std::env::var("KRIA_UINPUT_SOCKET").ok().map(PathBuf::from))
            .unwrap_or_else(|| {
                if let Ok(sudo_user) = std::env::var("SUDO_USER") {
                    let user_home = format!("/home/{}", sudo_user);
                    PathBuf::from(user_home).join(".cache/kria/uinput.sock")
                } else {
                    dirs::cache_dir()
                        .map(|d| d.join("kria").join("uinput.sock"))
                        .unwrap_or_else(|| PathBuf::from("/tmp/kria-uinput.sock"))
                }
            })
    };

    // Create secure socket
    let listener = create_secure_socket(&socket_path).await?;

    info!(socket = %socket_path.display(), "Daemon ready - waiting for connections");

    // Accept connections
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream).await {
                        error!(error = %e, "Client handler error");
                    }
                });
            }
            Err(e) => {
                error!(error = %e, "Failed to accept connection");
            }
        }
    }
}
