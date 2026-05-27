//! GUI System Control - Phase 1: Mechanical Bridge & Immune System
//!
//! RFC 007 Implementation: Atomic GUI automation with safety boundaries.
//! This module provides host-level GUI control through a privilege-isolated
//! architecture communicating via IPC to a minimal ydotool helper process.

use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

// ============================================================================
// Section 1: Backend Abstraction & IPC Architecture
// ============================================================================

/// Trait abstracting OS-level GUI input backends.
/// Implementations must be Send + Sync for use across async boundaries.
#[async_trait]
pub trait GuiBackend: Send + Sync {
    /// Execute a mouse click at the specified coordinates.
    async fn click_mouse(&self, x: i32, y: i32, button: MouseButton) -> Result<(), GuiError>;

    /// Type text with optional inter-keystroke interval.
    async fn type_text(&self, text: &str, interval_ms: Option<u64>) -> Result<(), GuiError>;

    /// Press a key combination (shortcut).
    async fn press_shortcut(
        &self,
        keys: &[Key],
        hold_duration_ms: Option<u64>,
    ) -> Result<(), GuiError>;

    /// Release all modifier keys (used for kill switch teardown).
    async fn release_all_modifiers(&self) -> Result<(), GuiError>;

    /// Activate the current active window to ensure keyboard focus (X11).
    async fn focus_window(&self) -> Result<(), GuiError>;

    /// Get current active window information for verification.
    async fn get_active_window(&self) -> Result<WindowInfo, GuiError>;

    /// RFC 008: Send heartbeat to uinput daemon for dead-man's switch.
    /// Must be called every 2-3 seconds to prevent daemon from halting input.
    async fn send_heartbeat(&self) -> Result<(), GuiError>;

    /// RFC 008: Explicitly signal task completion to daemon.
    /// This releases held keys and marks the disconnect as clean.
    async fn send_task_complete(&self) -> Result<(), GuiError>;
}

/// Mouse button variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Key variants for keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    // Modifier keys
    Shift,
    Control,
    Alt,
    Super,
    // Function keys
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    // Special keys
    Escape,
    Enter,
    Tab,
    Space,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    // Alphanumeric (use char for these)
    Char(char),
}

/// Information about the active window.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub title: String,
    pub class: String,
    pub pid: u32,
}

/// Errors from GUI backend operations.
#[derive(Debug, thiserror::Error)]
pub enum GuiError {
    #[error("IPC communication failed: {0}")]
    IpcError(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Invalid coordinates: ({0}, {1})")]
    InvalidCoordinates(i32, i32),
    #[error("Unknown key: {0}")]
    UnknownKey(String),
    #[error("Backend not available: {0}")]
    NotAvailable(String),
    #[error("Operation cancelled")]
    Cancelled,
}

// ============================================================================
// Ydotool Backend with IPC Isolation
// ============================================================================

/// Ydotool backend communicating via Unix Domain Socket to isolated helper.
///
/// Architecture: KRIA core (unprivileged) -> IPC socket -> ydotool-helper (uinput access)
/// This ensures the main process never requires elevated privileges.
pub struct YdotoolBackend {
    /// Path to the Unix Domain Socket for the helper process.
    socket_path: std::path::PathBuf,
}

/// IPC protocol request types (matching daemon protocol).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum IpcRequest {
    Click {
        x: i32,
        y: i32,
        button: String,
    },
    Type {
        text: String,
        interval_ms: Option<u64>,
    },
    Shortcut {
        keys: Vec<String>,
        hold_duration_ms: Option<u64>,
    },
    ReleaseAll,
    GetActiveWindow,
    /// RFC 008: Heartbeat for dead-man's switch
    Heartbeat,
    /// RFC 008: Explicit task completion — tells daemon to release keys and treat disconnect as clean
    TaskComplete,
}

/// IPC protocol response types (matching daemon protocol).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IpcResponse {
    Ok {
        data: Option<serde_json::Value>,
    },
    Error {
        message: String,
        code: Option<String>,
    },
}

impl YdotoolBackend {
    /// Create new backend instance.
    pub fn new(socket_path: std::path::PathBuf) -> Self {
        Self { socket_path }
    }

    /// Calculate dynamic timeout based on request type.
    /// Type commands need longer timeouts to allow the daemon to finish typing
    /// long text. Formula: base_timeout + chars * (interval + 5ms safety margin)
    fn calculate_timeout(request: &IpcRequest) -> Duration {
        const BASE_TIMEOUT_SECS: u64 = 10;
        const MAX_TIMEOUT_SECS: u64 = 120;

        match request {
            IpcRequest::Type { text, interval_ms } => {
                let interval = interval_ms.unwrap_or(10);
                // Calculate expected typing duration + safety margin
                let typing_ms = text.chars().count() as u64 * (interval + 5);
                let total_ms = (BASE_TIMEOUT_SECS * 1000) + typing_ms;
                Duration::from_millis(total_ms.min(MAX_TIMEOUT_SECS * 1000))
            }
            _ => Duration::from_secs(BASE_TIMEOUT_SECS),
        }
    }

    /// Send IPC command to daemon via Unix Domain Socket.
    async fn send_ipc_request(&self, request: &IpcRequest) -> Result<IpcResponse, GuiError> {
        // RFC 008 FIX: Dynamic timeout - Type commands need longer for big payloads
        let timeout = Self::calculate_timeout(request);

        tracing::debug!(target: "ydotool_ipc", 
            "Connecting to daemon at {:?} (timeout: {}ms)", 
            self.socket_path, timeout.as_millis());

        // Connect to daemon with short connection timeout (independent of read timeout)
        let connect_timeout = Duration::from_secs(10);
        let stream = tokio::time::timeout(
            connect_timeout,
            tokio::net::UnixStream::connect(&self.socket_path),
        )
        .await
        .map_err(|_| GuiError::IpcError("Connection timeout".to_string()))?
        .map_err(|e| GuiError::IpcError(format!("Failed to connect to daemon: {}", e)))?;

        let (reader, mut writer) = stream.into_split();

        // Serialize and send request
        let request_json = serde_json::to_string(request)
            .map_err(|e| GuiError::IpcError(format!("Failed to serialize request: {}", e)))?;

        tracing::debug!(target: "ydotool_ipc", "Sending: {}", request_json);

        // Write timeouts use connection timeout (write should be fast)
        tokio::time::timeout(connect_timeout, writer.write_all(request_json.as_bytes()))
            .await
            .map_err(|_| GuiError::IpcError("Write timeout".to_string()))?
            .map_err(|e| GuiError::IpcError(format!("Failed to write: {}", e)))?;

        tokio::time::timeout(connect_timeout, writer.write_all(b"\n"))
            .await
            .map_err(|_| GuiError::IpcError("Write newline timeout".to_string()))?
            .map_err(|e| GuiError::IpcError(format!("Failed to write newline: {}", e)))?;

        tokio::time::timeout(connect_timeout, writer.flush())
            .await
            .map_err(|_| GuiError::IpcError("Flush timeout".to_string()))?
            .map_err(|e| GuiError::IpcError(format!("Failed to flush: {}", e)))?;

        // FIX #32: Shut down the write half before reading the response.
        // Without this, daemons that use read_to_end() will block waiting for EOF
        // because the write half is still open. Shutting down signals EOF to the daemon.
        tokio::time::timeout(connect_timeout, writer.shutdown())
            .await
            .map_err(|_| GuiError::IpcError("Shutdown timeout".to_string()))?
            .map_err(|e| GuiError::IpcError(format!("Failed to shutdown write half: {}", e)))?;

        // Read response line - uses DYNAMIC timeout for slow Type operations
        let mut buf_reader = tokio::io::BufReader::new(reader);
        let mut response_line = String::new();

        tokio::time::timeout(timeout, buf_reader.read_line(&mut response_line))
            .await
            .map_err(|_| GuiError::IpcError(format!("Read timeout after {}s", timeout.as_secs())))?
            .map_err(|e| GuiError::IpcError(format!("Failed to read response: {}", e)))?;

        tracing::debug!(target: "ydotool_ipc", "Received: {}", response_line.trim());

        // Parse response
        let response: IpcResponse = serde_json::from_str(&response_line)
            .map_err(|e| GuiError::IpcError(format!("Failed to parse response: {}", e)))?;

        Ok(response)
    }

    /// Execute command and return success or error.
    async fn execute_command(&self, request: IpcRequest) -> Result<(), GuiError> {
        match self.send_ipc_request(&request).await? {
            IpcResponse::Ok { .. } => Ok(()),
            IpcResponse::Error { message, code } => Err(GuiError::IpcError(format!(
                "{}: {}",
                code.unwrap_or_else(|| "DAEMON_ERROR".to_string()),
                message
            ))),
        }
    }
}

/// RFC 008: Check global safety halt at backend layer (defense in depth).
/// Returns an error if automation is globally disabled, with the actual reason
/// (e.g. "vision sidecar starting", "user disabled automation").
#[inline]
fn check_global_halt() -> Result<(), GuiError> {
    if crate::safety::is_halted() {
        let reason = crate::safety::halt_reason().unwrap_or_else(|| "unknown".to_string());
        Err(GuiError::IpcError(format!("GLOBAL_SAFETY_HALT: {reason}")))
    } else {
        Ok(())
    }
}

#[async_trait]
impl GuiBackend for YdotoolBackend {
    async fn click_mouse(&self, x: i32, y: i32, button: MouseButton) -> Result<(), GuiError> {
        check_global_halt()?;

        if x < 0 || y < 0 {
            return Err(GuiError::InvalidCoordinates(x, y));
        }

        let button_str = match button {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
        };

        let request = IpcRequest::Click {
            x,
            y,
            button: button_str.to_string(),
        };

        self.execute_command(request).await
    }

    async fn type_text(&self, text: &str, interval_ms: Option<u64>) -> Result<(), GuiError> {
        check_global_halt()?;

        let request = IpcRequest::Type {
            text: text.to_string(),
            interval_ms,
        };

        self.execute_command(request).await
    }

    async fn press_shortcut(
        &self,
        keys: &[Key],
        hold_duration_ms: Option<u64>,
    ) -> Result<(), GuiError> {
        check_global_halt()?;

        let key_strings: Vec<String> = keys
            .iter()
            .map(|k| key_to_ydotool(k))
            .collect::<Result<Vec<_>, _>>()?;

        let request = IpcRequest::Shortcut {
            keys: key_strings,
            hold_duration_ms,
        };

        self.execute_command(request).await
    }

    async fn release_all_modifiers(&self) -> Result<(), GuiError> {
        let request = IpcRequest::ReleaseAll;

        // Best effort - don't fail if release fails
        let _ = self.execute_command(request).await;
        Ok(())
    }

    async fn send_heartbeat(&self) -> Result<(), GuiError> {
        let request = IpcRequest::Heartbeat;
        self.execute_command(request).await
    }

    async fn send_task_complete(&self) -> Result<(), GuiError> {
        let request = IpcRequest::TaskComplete;
        // Best effort — don't fail workflow if daemon doesn't support TaskComplete yet
        let _ = self.execute_command(request).await;
        Ok(())
    }

    async fn focus_window(&self) -> Result<(), GuiError> {
        // Use xdotool to activate the active window
        // This ensures keyboard focus in X11
        // Execute: xdotool windowactiv $(xdotool getactivewindow)
        let window_id = tokio::process::Command::new("xdotool")
            .args(["getactivewindow"])
            .output()
            .await
            .map_err(|e| {
                GuiError::IpcError(format!("Failed to execute xdotool getactivewindow: {}", e))
            })?;

        if !window_id.status.success() {
            let stderr = String::from_utf8_lossy(&window_id.stderr);
            return Err(GuiError::IpcError(format!(
                "xdotool getactivewindow failed: {}",
                stderr
            )));
        }

        let window_id_str = String::from_utf8_lossy(&window_id.stdout)
            .trim()
            .to_string();
        if window_id_str.is_empty() {
            return Err(GuiError::IpcError("No active window found".to_string()));
        }

        let output = tokio::process::Command::new("xdotool")
            .args(["windowactivate", &window_id_str])
            .output()
            .await
            .map_err(|e| {
                GuiError::IpcError(format!("Failed to execute xdotool windowactivate: {}", e))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GuiError::IpcError(format!(
                "xdotool windowactivate failed: {}",
                stderr
            )));
        }

        Ok(())
    }

    async fn get_active_window(&self) -> Result<WindowInfo, GuiError> {
        let request = IpcRequest::GetActiveWindow;

        match self.send_ipc_request(&request).await {
            Ok(IpcResponse::Ok { data }) => {
                if let Some(data) = data {
                    // Parse WindowInfo from response
                    let title = data["title"].as_str().unwrap_or("Unknown").to_string();
                    let class = data["class"].as_str().unwrap_or("Unknown").to_string();
                    let pid = data["pid"].as_u64().unwrap_or(0) as u32;
                    return Ok(WindowInfo { title, class, pid });
                }
                Err(GuiError::IpcError("No window data in response".to_string()))
            }
            Ok(IpcResponse::Error { message, code }) => Err(GuiError::IpcError(format!(
                "{}: {}",
                code.unwrap_or_else(|| "WINDOW_ERROR".to_string()),
                message
            ))),
            Err(e) => {
                // IPC failed (WINDOW_ID_FAILED on Wayland, daemon not running, etc.)
                // Try Wayland-native fallback: AT-SPI via /proc + xdg-activation heuristic.
                // This gives us the focused app's process name even without a window server.
                tracing::debug!(
                    target: "gui_automation",
                    ipc_error = %e,
                    "IPC get_active_window failed — trying Wayland-native fallback"
                );
                Self::get_active_window_wayland_fallback()
                    .await
                    .map_err(|fallback_err| {
                        // Both paths failed — return the original IPC error with fallback note
                        GuiError::IpcError(format!(
                            "{} (Wayland fallback also failed: {})",
                            e, fallback_err
                        ))
                    })
            }
        }
    }
}

/// Wayland-native fallback methods for `YdotoolBackend`.
/// These are not part of the `GuiBackend` trait — they are internal helpers
/// used when the uinput daemon IPC fails on Wayland.
impl YdotoolBackend {
    /// Wayland-native fallback for active window detection.
    ///
    /// When the uinput daemon IPC fails (WINDOW_ID_FAILED on Wayland), this
    /// method tries to determine the focused application using:
    /// 1. AT-SPI D-Bus: query the accessibility bus for the focused application
    /// 2. /proc heuristic: find the most recently started GUI process
    ///
    /// This is best-effort — it may return stale or approximate data.
    /// The result is used for the safety guard (runaway prevention), not for
    /// user-visible output, so approximate data is acceptable.
    async fn get_active_window_wayland_fallback() -> Result<WindowInfo, String> {
        // Strategy 1: AT-SPI via D-Bus
        // The AT-SPI bus address is available via org.a11y.Bus on the session bus.
        if let Ok(info) = Self::get_window_via_atspi().await {
            return Ok(info);
        }

        // Strategy 2: /proc heuristic — find the most recently started GUI process
        // by scanning /proc for processes with a DISPLAY or WAYLAND_DISPLAY env var.
        // This is a last resort and may be inaccurate.
        if let Some(info) = Self::get_window_via_proc_heuristic() {
            return Ok(info);
        }

        Err("All Wayland fallback strategies exhausted".to_string())
    }

    /// Try to get the focused application via AT-SPI D-Bus.
    async fn get_window_via_atspi() -> Result<WindowInfo, String> {
        // Get the AT-SPI bus address from the session bus
        let session_bus = zbus::Connection::session()
            .await
            .map_err(|e| format!("Cannot connect to session bus: {}", e))?;

        let atspi_address: String = session_bus
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.a11y.Bus"),
                "GetAddress",
                &(),
            )
            .await
            .map_err(|e| format!("Cannot get AT-SPI bus address: {}", e))?
            .body()
            .deserialize()
            .map_err(|e| format!("Cannot deserialize AT-SPI address: {}", e))?;

        // Connect to the AT-SPI bus
        let atspi_bus = zbus::ConnectionBuilder::address(atspi_address.as_str())
            .map_err(|e| format!("Invalid AT-SPI address: {}", e))?
            .build()
            .await
            .map_err(|e| format!("Cannot connect to AT-SPI bus: {}", e))?;

        // Query the focused application from the AT-SPI registry
        // The focused object is available via org.a11y.atspi.Registry.GetFocusedObject
        // (not always available) or by iterating applications.
        // We use a simpler approach: query the desktop's children for the focused app.
        let focused_app: (String, zbus::zvariant::OwnedObjectPath) = atspi_bus
            .call_method(
                Some("org.a11y.atspi.Registry"),
                "/org/a11y/atspi/registry",
                Some("org.a11y.atspi.Registry"),
                "GetFocusedObject",
                &(),
            )
            .await
            .map_err(|e| format!("GetFocusedObject failed: {}", e))?
            .body()
            .deserialize()
            .map_err(|e| format!("Cannot deserialize focused object: {}", e))?;

        let (app_bus, app_path) = focused_app;

        // Get the application name from the focused object's parent application
        let app_name: String = atspi_bus
            .call_method(
                Some(app_bus.as_str()),
                app_path.as_str(),
                Some("org.a11y.atspi.Accessible"),
                "GetApplication",
                &(),
            )
            .await
            .ok()
            .and_then(|msg| msg.body().deserialize::<String>().ok())
            .unwrap_or_else(|| app_bus.clone());

        tracing::debug!(
            target: "gui_automation",
            app_name = %app_name,
            "AT-SPI: got focused application"
        );

        Ok(WindowInfo {
            title: app_name.clone(),
            class: app_name,
            pid: 0, // AT-SPI doesn't easily give us PID without more queries
        })
    }

    /// Last-resort heuristic: scan /proc for the most recently started GUI process.
    fn get_window_via_proc_heuristic() -> Option<WindowInfo> {
        // Find processes that have WAYLAND_DISPLAY or DISPLAY in their environment,
        // are not system processes, and were started most recently.
        // This is approximate but better than nothing for the safety guard.
        let mut candidates: Vec<(u64, String)> = Vec::new(); // (start_time, name)

        if let Ok(entries) = std::fs::read_dir("/proc") {
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

                // Check if this process has a GUI environment
                let environ_path = pid_dir.join("environ");
                let Ok(environ) = std::fs::read(&environ_path) else {
                    continue;
                };
                let has_display = environ.windows(8).any(|w| w == b"DISPLAY=")
                    || environ.windows(17).any(|w| w == b"WAYLAND_DISPLAY=");
                if !has_display {
                    continue;
                }

                // Get process name and start time
                let comm_path = pid_dir.join("comm");
                let Ok(comm) = std::fs::read_to_string(&comm_path) else {
                    continue;
                };
                let comm = comm.trim().to_string();

                // Skip kernel threads and common system processes
                if comm.starts_with('[')
                    || matches!(
                        comm.as_str(),
                        "systemd" | "dbus-daemon" | "pulseaudio" | "pipewire" | "Xwayland"
                    )
                {
                    continue;
                }

                // Use inode of /proc/<pid> as a proxy for start time
                if let Ok(meta) = std::fs::metadata(&pid_dir) {
                    use std::os::unix::fs::MetadataExt;
                    candidates.push((meta.ino(), comm));
                }
            }
        }

        // Sort by inode descending (higher inode = more recently created)
        candidates.sort_by(|a, b| b.0.cmp(&a.0));

        candidates.first().map(|(_, name)| {
            tracing::debug!(
                target: "gui_automation",
                process = %name,
                "Wayland fallback: using most recent GUI process as active window heuristic"
            );
            WindowInfo {
                title: name.clone(),
                class: name.clone(),
                pid: 0,
            }
        })
    }
}

/// Convert Key enum to ydotool key names.
fn key_to_ydotool(key: &Key) -> Result<String, GuiError> {
    let name = match key {
        Key::Shift => "Shift",
        Key::Control => "Control",
        Key::Alt => "Alt",
        Key::Super => "Super",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        Key::Escape => "Escape",
        Key::Enter => "Return",
        Key::Tab => "Tab",
        Key::Space => "Space",
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "Page_Up",
        Key::PageDown => "Page_Down",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::Char(c) => return Ok(c.to_string()),
    };
    Ok(name.to_string())
}

// ============================================================================
// Section 2: Clipboard Atomic Backup System
// ============================================================================

/// Thread-safe atomic clipboard backup held in main thread scope.
///
/// This ensures clipboard restoration even if the agent async task panics
/// or is killed mid-typing, preventing permanent data loss.
pub struct ClipboardAtomicBackup {
    /// The backed-up clipboard content (if any).
    content: RwLock<Option<String>>,
    /// Whether a backup is currently active.
    has_backup: RwLock<bool>,
}

impl ClipboardAtomicBackup {
    pub fn new() -> Self {
        Self {
            content: RwLock::new(None),
            has_backup: RwLock::new(false),
        }
    }

    /// Create atomic backup of current clipboard.
    /// Must be called before any clipboard-modifying operation.
    pub async fn backup(&self) -> Result<(), String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("Failed to access clipboard: {}", e))?;

        let text = clipboard
            .get_text()
            .map_err(|e| format!("Failed to read clipboard: {}", e))?;

        let mut content = self.content.write().await;
        *content = Some(text);

        let mut has_backup = self.has_backup.write().await;
        *has_backup = true;

        tracing::debug!(target: "clipboard_backup", "Clipboard backed up ({} chars)", 
            content.as_ref().map(|s| s.len()).unwrap_or(0));

        Ok(())
    }

    /// Restore clipboard to backed-up state.
    /// Safe to call multiple times - subsequent calls are no-ops if already restored.
    pub async fn restore(&self) -> Result<(), String> {
        let has_backup = *self.has_backup.read().await;
        if !has_backup {
            return Ok(()); // Nothing to restore
        }

        let content = self.content.read().await;
        if let Some(text) = content.as_ref() {
            let mut clipboard = arboard::Clipboard::new()
                .map_err(|e| format!("Failed to access clipboard for restore: {}", e))?;

            clipboard
                .set_text(text.clone())
                .map_err(|e| format!("Failed to restore clipboard: {}", e))?;

            tracing::debug!(target: "clipboard_backup", "Clipboard restored ({} chars)", text.len());
        }

        // Clear backup state
        drop(content);
        let mut has_backup = self.has_backup.write().await;
        *has_backup = false;

        Ok(())
    }

    /// Clear backup without restoring (used when operation succeeds).
    pub async fn clear(&self) {
        let mut content = self.content.write().await;
        *content = None;
        let mut has_backup = self.has_backup.write().await;
        *has_backup = false;
    }
}

impl Default for ClipboardAtomicBackup {
    fn default() -> Self {
        Self::new()
    }
}

// Global atomic backup instance (lives in main thread scope)
static CLIPBOARD_BACKUP: Lazy<ClipboardAtomicBackup> = Lazy::new(ClipboardAtomicBackup::new);

// ============================================================================
// Section 3: Kill Switch Interceptor with Rate Limiting
// ============================================================================

/// Interceptor providing kill switch, rate limiting, and teardown safety.
///
/// This struct wraps all GUI tool execution and enforces:
/// - Cancellation token checks before every action
/// - Hard rate limiting (max 2 actions/sec, min 500ms delay)
/// - Modifier key release on termination (prevents OS lockup)
pub struct KillSwitchInterceptor {
    /// Cancellation token for this operation sequence.
    cancellation: CancellationToken,
    /// Backend for executing modifier release on teardown.
    backend: Arc<dyn GuiBackend>,
    /// Rate limiter state.
    last_action: Mutex<Option<Instant>>,
    /// Minimum delay between actions (500ms).
    min_delay: Duration,
    /// Maximum actions per second (2).
    max_rate: u32,
    /// Action counter for current second.
    action_count: Mutex<u32>,
    /// Window for rate counting.
    rate_window_start: Mutex<Instant>,
    /// RFC v2 (F8): Whether this session has ever issued a shortcut (modifier).
    /// Used to make `release_all_modifiers` idempotent so periodic teardowns
    /// without any prior modifier press do not spam the daemon.
    modifier_was_pressed: std::sync::atomic::AtomicBool,
}

impl KillSwitchInterceptor {
    /// Create new interceptor.
    pub fn new(cancellation: CancellationToken, backend: Arc<dyn GuiBackend>) -> Self {
        Self {
            cancellation,
            backend,
            last_action: Mutex::new(None),
            min_delay: Duration::from_millis(500),
            max_rate: 2,
            action_count: Mutex::new(0),
            rate_window_start: Mutex::new(Instant::now()),
            modifier_was_pressed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// RFC v2 (F8): Notify the kill switch that a shortcut/modifier was issued.
    /// Call this from any code path that asks the backend to press a modifier
    /// (Shift/Control/Alt/Super). The teardown will then perform a real
    /// release; otherwise it short-circuits.
    pub fn mark_modifier_pressed(&self) {
        self.modifier_was_pressed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// RFC v2 (F8): Whether any modifier was ever pressed in this session.
    /// Used by callers (e.g. `GuiExecutor`) to skip redundant ReleaseAll calls.
    pub fn modifier_was_pressed(&self) -> bool {
        self.modifier_was_pressed
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Check cancellation and rate limits before executing action.
    /// Returns Err if operation should not proceed.
    pub async fn check_preconditions(&self) -> Result<(), GuiError> {
        // Check cancellation first
        if self.cancellation.is_cancelled() {
            self.execute_teardown().await;
            return Err(GuiError::Cancelled);
        }

        // Enforce rate limiting
        self.enforce_rate_limit().await?;

        Ok(())
    }

    /// Enforce hard rate limits per RFC 007.
    /// - Max 2 actions per second
    /// - Min 500ms between actions
    async fn enforce_rate_limit(&self) -> Result<(), GuiError> {
        let now = Instant::now();

        // Check minimum delay between actions
        let mut last = self.last_action.lock().await;
        if let Some(last_time) = *last {
            let elapsed = now.duration_since(last_time);
            if elapsed < self.min_delay {
                let wait = self.min_delay - elapsed;
                tracing::debug!(target: "rate_limit", "Rate limiting: waiting {}ms", wait.as_millis());
                tokio::time::sleep(wait).await;
            }
        }
        *last = Some(Instant::now());
        drop(last);

        // Check max actions per second
        let mut window_start = self.rate_window_start.lock().await;
        let mut count = self.action_count.lock().await;

        if now.duration_since(*window_start) >= Duration::from_secs(1) {
            // New window
            *window_start = now;
            *count = 1;
        } else if *count >= self.max_rate {
            // Rate exceeded - wait for next window
            let wait = Duration::from_secs(1) - now.duration_since(*window_start);
            tracing::debug!(target: "rate_limit", "Rate limit exceeded, waiting {}ms for next window",
                wait.as_millis());
            drop(count);
            drop(window_start);
            tokio::time::sleep(wait).await;

            // Reset for new window
            let mut window_start = self.rate_window_start.lock().await;
            let mut count = self.action_count.lock().await;
            *window_start = Instant::now();
            *count = 1;
        } else {
            *count += 1;
        }

        Ok(())
    }

    /// Get reference to the GUI backend.
    pub fn get_backend(&self) -> Arc<dyn GuiBackend> {
        Arc::clone(&self.backend)
    }

    /// Execute teardown sequence: release all modifier keys.
    /// This prevents OS keyboard lockup when agent is killed.
    ///
    /// RFC v2 (F8): If no modifier was ever pressed during this session, the
    /// release call is skipped entirely. Eliminates the noisy `xdotool keyup`
    /// burst that previously followed every clean workflow.
    pub async fn execute_teardown(&self) {
        if !self
            .modifier_was_pressed
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            tracing::debug!(
                target: "kill_switch",
                "Teardown skipped — no modifier was pressed this session"
            );
            // Clipboard backup still restored even if no modifiers were used:
            // some workflows may have replaced clipboard without modifiers.
            if let Err(e) = CLIPBOARD_BACKUP.restore().await {
                tracing::error!(target: "kill_switch", "Failed to restore clipboard: {}", e);
            }
            return;
        }

        tracing::warn!(target: "kill_switch", "Executing kill switch teardown - releasing modifiers");

        // Release all modifier keys to prevent stuck keys
        if let Err(e) = self.backend.release_all_modifiers().await {
            tracing::error!(target: "kill_switch", "Failed to release modifiers: {}", e);
        }

        // Restore clipboard if backup exists
        if let Err(e) = CLIPBOARD_BACKUP.restore().await {
            tracing::error!(target: "kill_switch", "Failed to restore clipboard: {}", e);
        }
    }
}

impl Drop for KillSwitchInterceptor {
    fn drop(&mut self) {
        // RFC v2 (F8): Skip Drop teardown if no modifier was ever pressed.
        if !self
            .modifier_was_pressed
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        // FIX #4: Guard against spawning into a dead/shutdown runtime.
        // tokio::spawn panics if called outside a Tokio runtime context
        // (e.g., during test teardown or after runtime shutdown).
        // Use Handle::try_current() to check before spawning.
        let backend = Arc::clone(&self.backend);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                // Runtime is alive — spawn the cleanup task.
                handle.spawn(async move {
                    if let Err(e) = backend.release_all_modifiers().await {
                        tracing::error!(target: "kill_switch", "Drop teardown failed: {}", e);
                    }
                });
            }
            Err(_) => {
                // No runtime available (shutdown, test teardown, non-Tokio thread).
                // Log and skip — modifier keys may remain stuck but we cannot
                // safely spawn async work here.
                tracing::warn!(
                    target: "kill_switch",
                    "Drop teardown skipped: no Tokio runtime available. \
                     Modifier keys may be stuck if a workflow was interrupted."
                );
            }
        }
    }
}

// ============================================================================
// Section 4: Active Window Verification & Protected Mode
// ============================================================================

/// Protected Mode detection using allowlists/blocklists.
///
/// Per RFC 007 Section 2.4, this prevents automation in sensitive contexts
/// like password managers, banking sites, and system auth dialogs.
pub struct ProtectedModeDetector {
    /// Blocked window titles (case-insensitive substring match).
    blocked_titles: HashSet<String>,
    /// Blocked window classes.
    blocked_classes: HashSet<String>,
    /// Blocked URL patterns (for browser windows).
    #[allow(dead_code)] // Reserved for future browser context detection
    blocked_urls: HashSet<String>,
    /// Allowed window titles (if non-empty, only these are allowed).
    allowed_titles: HashSet<String>,
}

impl ProtectedModeDetector {
    pub fn new() -> Self {
        let mut detector = Self {
            blocked_titles: HashSet::new(),
            blocked_classes: HashSet::new(),
            blocked_urls: HashSet::new(),
            allowed_titles: HashSet::new(),
        };

        // Initialize with default blocklist per RFC 007
        detector.initialize_defaults();
        detector
    }

    fn initialize_defaults(&mut self) {
        // Password managers
        self.blocked_titles.insert("KeePass".to_lowercase());
        self.blocked_titles.insert("1Password".to_lowercase());
        self.blocked_titles.insert("LastPass".to_lowercase());
        self.blocked_titles.insert("Bitwarden".to_lowercase());
        self.blocked_titles.insert("Dashlane".to_lowercase());
        self.blocked_titles.insert("password".to_lowercase()); // Generic heuristic

        // Banking sites (browser titles often contain these)
        self.blocked_titles.insert("chase.com".to_lowercase());
        self.blocked_titles.insert("wellsfargo.com".to_lowercase());
        self.blocked_titles
            .insert("bankofamerica.com".to_lowercase());
        self.blocked_titles.insert("citi.com".to_lowercase());
        self.blocked_titles.insert("usbank.com".to_lowercase());
        self.blocked_titles.insert("paypal.com".to_lowercase());

        // System auth
        self.blocked_titles.insert("sudo".to_lowercase());
        self.blocked_titles.insert("authentication".to_lowercase());
        self.blocked_titles
            .insert("password required".to_lowercase());
        self.blocked_titles.insert("unlock".to_lowercase());

        // Blocked classes
        self.blocked_classes.insert("pinentry".to_lowercase());
        self.blocked_classes.insert("polkit".to_lowercase());
        self.blocked_classes.insert("gcr-prompter".to_lowercase());
    }

    /// Check if automation should be blocked for the given window.
    pub fn is_protected(&self, window: &WindowInfo) -> bool {
        let title_lower = window.title.to_lowercase();
        let class_lower = window.class.to_lowercase();

        // Check blocklists
        for blocked in &self.blocked_titles {
            if title_lower.contains(blocked) {
                tracing::warn!(target: "protected_mode", 
                    "Protected mode triggered by title: '{}' (matched: '{}')", 
                    window.title, blocked);
                return true;
            }
        }

        for blocked in &self.blocked_classes {
            if class_lower.contains(blocked) {
                tracing::warn!(target: "protected_mode",
                    "Protected mode triggered by class: '{}' (matched: '{}')",
                    window.class, blocked);
                return true;
            }
        }

        // If allowlist is populated, check it
        if !self.allowed_titles.is_empty() {
            let allowed = self
                .allowed_titles
                .iter()
                .any(|allowed| title_lower.contains(allowed));
            if !allowed {
                tracing::warn!(target: "protected_mode",
                    "Window '{}' not in allowlist, blocking", window.title);
                return true;
            }
        }

        false
    }

    /// Verify active window matches expected context before input.
    pub async fn verify_active_window(
        &self,
        backend: &dyn GuiBackend,
        expected_title: Option<&str>,
    ) -> Result<WindowInfo, GuiError> {
        let window = backend.get_active_window().await?;

        // Check protected mode
        if self.is_protected(&window) {
            return Err(GuiError::PermissionDenied(format!(
                "Protected mode active for window: '{}' (class: '{}')",
                window.title, window.class
            )));
        }

        // Verify expected title if provided
        if let Some(expected) = expected_title {
            let window_lower = window.title.to_lowercase();
            let expected_lower = expected.to_lowercase();

            if !window_lower.contains(&expected_lower) {
                return Err(GuiError::PermissionDenied(format!(
                    "Active window '{}' does not match expected '{}'",
                    window.title, expected
                )));
            }
        }

        Ok(window)
    }
}

impl Default for ProtectedModeDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Section 5: Tool Implementations (RED Tier)
// ============================================================================

/// Control character blacklist for shell safety.
/// Per RFC 007, these require explicit HITL approval in terminals.
const SHELL_CONTROL_CHARS: &[char] = &['\n', '|', '>', '<', '&', ';', '$', '`', '\\'];

/// Shared state for GUI tool handlers.
struct GuiToolState {
    backend: Arc<dyn GuiBackend>,
    detector: ProtectedModeDetector,
}

/// click_mouse tool implementation.
struct ClickMouse {
    state: Arc<GuiToolState>,
}

#[async_trait]
impl ToolHandler for ClickMouse {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        // Parse parameters
        let x = match params["x"].as_i64() {
            Some(v) => v as i32,
            None => return ToolResult::err("Missing x parameter"),
        };
        let y = match params["y"].as_i64() {
            Some(v) => v as i32,
            None => return ToolResult::err("Missing y parameter"),
        };
        let button = params["button"].as_str().unwrap_or("left");

        let button = match button {
            "left" => MouseButton::Left,
            "right" => MouseButton::Right,
            "middle" => MouseButton::Middle,
            _ => return ToolResult::err(format!("Invalid button: {}", button)),
        };

        // Create interceptor (kill switch + rate limiting)
        let cancellation = CancellationToken::new();
        let interceptor = KillSwitchInterceptor::new(cancellation, Arc::clone(&self.state.backend));

        // Check preconditions
        if let Err(e) = interceptor.check_preconditions().await {
            return ToolResult::err(format!("Precondition check failed: {}", e));
        }

        // Verify active window (no protected mode)
        if let Err(e) = self
            .state
            .detector
            .verify_active_window(self.state.backend.as_ref(), None)
            .await
        {
            return ToolResult::err(format!("Window verification failed: {}", e));
        }

        // Execute
        // Serialize button as string for JSON
        let button_str = match button {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
        };

        match self.state.backend.click_mouse(x, y, button).await {
            Ok(_) => ToolResult::ok(serde_json::json!({
                "clicked": true,
                "x": x,
                "y": y,
                "button": button_str
            })),
            Err(e) => ToolResult::err(format!("Click failed: {}", e)),
        }
    }
}

/// type_text tool implementation with clipboard atomic backup.
struct TypeText {
    state: Arc<GuiToolState>,
}

#[async_trait]
impl ToolHandler for TypeText {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let text = match params["text"].as_str() {
            Some(s) => s,
            None => return ToolResult::err("Missing text parameter"),
        };
        let interval_ms = params["interval_ms"].as_u64();
        let expected_window = params["expected_window"].as_str();

        // Check for control characters (terminal safety)
        let is_terminal = params["is_terminal"].as_bool().unwrap_or(false);
        if is_terminal {
            for ch in text.chars() {
                if SHELL_CONTROL_CHARS.contains(&ch) {
                    return ToolResult::err(format!(
                        "Control character '{}' detected in terminal context. \
                        Requires explicit HITL approval per RFC 007.",
                        ch
                    ));
                }
            }
        }

        // Create interceptor
        let cancellation = CancellationToken::new();
        let interceptor = KillSwitchInterceptor::new(cancellation, Arc::clone(&self.state.backend));

        if let Err(e) = interceptor.check_preconditions().await {
            return ToolResult::err(format!("Precondition check failed: {}", e));
        }

        // Verify active window
        if let Err(e) = self
            .state
            .detector
            .verify_active_window(self.state.backend.as_ref(), expected_window)
            .await
        {
            return ToolResult::err(format!("Window verification failed: {}", e));
        }

        // Atomic clipboard backup
        if let Err(e) = CLIPBOARD_BACKUP.backup().await {
            tracing::warn!(target: "type_text", "Failed to backup clipboard: {}", e);
            // Continue anyway - backup is best-effort safety
        }

        // Execute typing
        let result = match self.state.backend.type_text(text, interval_ms).await {
            Ok(_) => ToolResult::ok(serde_json::json!({
                "typed": true,
                "length": text.len(),
            })),
            Err(e) => {
                // Restore clipboard on failure
                if let Err(restore_err) = CLIPBOARD_BACKUP.restore().await {
                    tracing::error!(target: "type_text", "Failed to restore clipboard: {}", restore_err);
                }
                ToolResult::err(format!("Type failed: {}", e))
            }
        };

        // Clear backup on success (clipboard was intentionally modified)
        if result.success {
            CLIPBOARD_BACKUP.clear().await;
        }

        result
    }
}

/// press_shortcut tool implementation.
struct PressShortcut {
    state: Arc<GuiToolState>,
}

#[async_trait]
impl ToolHandler for PressShortcut {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let keys_json = match params["keys"].as_array() {
            Some(arr) => arr,
            None => return ToolResult::err("Missing keys parameter (array expected)"),
        };
        let hold_duration_ms = params["hold_duration_ms"].as_u64();

        // Parse key strings
        let mut keys: Vec<Key> = Vec::new();
        for key_json in keys_json {
            let key_str = match key_json.as_str() {
                Some(s) => s,
                None => return ToolResult::err("Keys must be strings"),
            };
            let key = match parse_key_string(key_str) {
                Ok(k) => k,
                Err(e) => return ToolResult::err(e),
            };
            keys.push(key);
        }

        // Check for dangerous combinations
        let has_dangerous = keys
            .iter()
            .any(|k| matches!(k, Key::Control | Key::Alt | Key::Super))
            && keys
                .iter()
                .any(|k| matches!(k, Key::Char('c') | Key::Char('x') | Key::Char('v')));

        if has_dangerous {
            // Log but allow - these are common shortcuts.
            // PolicyEngine should enforce RED tier PIN.
            tracing::info!(target: "press_shortcut", "Dangerous shortcut detected: {:?}", keys);
        }

        // Create interceptor
        let cancellation = CancellationToken::new();
        let interceptor = KillSwitchInterceptor::new(cancellation, Arc::clone(&self.state.backend));

        if let Err(e) = interceptor.check_preconditions().await {
            return ToolResult::err(format!("Precondition check failed: {}", e));
        }

        // Verify active window
        if let Err(e) = self
            .state
            .detector
            .verify_active_window(self.state.backend.as_ref(), None)
            .await
        {
            return ToolResult::err(format!("Window verification failed: {}", e));
        }

        // Execute shortcut
        match self
            .state
            .backend
            .press_shortcut(&keys, hold_duration_ms)
            .await
        {
            Ok(_) => ToolResult::ok(serde_json::json!({
                "pressed": true,
                "keys": keys_json.clone(),
            })),
            Err(e) => ToolResult::err(format!("Shortcut failed: {}", e)),
        }
    }
}

/// release_all tool implementation.
/// GREEN tier: harmless modifier release for pre-input sanitization.
struct ReleaseAll {
    state: Arc<GuiToolState>,
}

#[async_trait]
impl ToolHandler for ReleaseAll {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        match self.state.backend.release_all_modifiers().await {
            Ok(_) => ToolResult::ok(serde_json::json!({
                "released": true,
            })),
            Err(e) => ToolResult::err(format!("ReleaseAll failed: {}", e)),
        }
    }
}

/// focus_window tool implementation.
/// GREEN tier: harmless window activation for X11 focus.
struct FocusWindow {
    state: Arc<GuiToolState>,
}

#[async_trait]
impl ToolHandler for FocusWindow {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        match self.state.backend.focus_window().await {
            Ok(_) => ToolResult::ok(serde_json::json!({
                "focused": true,
            })),
            Err(e) => ToolResult::err(format!("FocusWindow failed: {}", e)),
        }
    }
}

/// system_sleep tool implementation.
/// GREEN tier: harmless sleep/wait for UI stabilization.
struct SystemSleep;

#[async_trait]
impl ToolHandler for SystemSleep {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let duration_ms = params["duration_ms"].as_u64().unwrap_or(1000);
        // AUDIT FIX #31: Cap sleep at 30 seconds to prevent LLM-generated
        // workflows from sleeping indefinitely (e.g., {"duration_ms": 86400000}).
        let duration_ms = duration_ms.min(30_000);
        tokio::time::sleep(Duration::from_millis(duration_ms)).await;
        ToolResult::ok(serde_json::json!({
            "slept": true,
            "duration_ms": duration_ms,
        }))
    }
}

/// Parse string representation into Key enum.
fn parse_key_string(s: &str) -> Result<Key, String> {
    let key = match s.to_lowercase().as_str() {
        "shift" => Key::Shift,
        "ctrl" | "control" => Key::Control,
        "alt" => Key::Alt,
        "super" | "win" | "cmd" | "command" => Key::Super,
        "escape" | "esc" => Key::Escape,
        "enter" | "return" => Key::Enter,
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
        // Single character
        c if c.len() == 1 => Key::Char(c.chars().next().unwrap()),
        _ => return Err(format!("Unknown key: {}", s)),
    };
    Ok(key)
}

// ============================================================================
// Section 6: Tool Registration (RED Tier)
// ============================================================================

pub fn register(reg: &ToolRegistry) {
    // Initialize GUI backend with socket path matching the kria-uinput-daemon
    let socket_path = crate::agent::gui_services::default_uinput_socket_path();
    let backend: Arc<dyn GuiBackend> = Arc::new(YdotoolBackend::new(socket_path));

    let state = Arc::new(GuiToolState {
        backend: Arc::clone(&backend),
        detector: ProtectedModeDetector::new(),
    });

    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "click_mouse".into(),
                description: "Click mouse at specified screen coordinates. \
                    Requires PIN confirmation (RED tier). \
                    Protected mode prevents clicks on password managers and banking sites.".into(),
                category: "gui_automation".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    ParamDef {
                        name: "x".into(),
                        param_type: "integer".into(),
                        description: "Horizontal screen coordinate (pixels)".into(),
                        required: true,
                        default: None,
                    },
                    ParamDef {
                        name: "y".into(),
                        param_type: "integer".into(),
                        description: "Vertical screen coordinate (pixels)".into(),
                        required: true,
                        default: None,
                    },
                    ParamDef {
                        name: "button".into(),
                        param_type: "string".into(),
                        description: "Mouse button: left, right, middle (default: left)".into(),
                        required: false,
                        default: Some(serde_json::json!("left")),
                    },
                ],
            },
            Arc::new(ClickMouse { state: Arc::clone(&state) }),
        ),
        (
            ToolDef {
                name: "type_text".into(),
                description: "Type text as keyboard input. \
                    Requires PIN confirmation (RED tier). \
                    Control characters blocked in terminal contexts. \
                    Clipboard state is atomically preserved.".into(),
                category: "gui_automation".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    ParamDef {
                        name: "text".into(),
                        param_type: "string".into(),
                        description: "Text to type".into(),
                        required: true,
                        default: None,
                    },
                    ParamDef {
                        name: "interval_ms".into(),
                        param_type: "integer".into(),
                        description: "Delay between keystrokes in milliseconds (optional)".into(),
                        required: false,
                        default: None,
                    },
                    ParamDef {
                        name: "expected_window".into(),
                        param_type: "string".into(),
                        description: "Expected active window title for verification (optional)".into(),
                        required: false,
                        default: None,
                    },
                    ParamDef {
                        name: "is_terminal".into(),
                        param_type: "boolean".into(),
                        description: "Whether target is a terminal (enables control char blocking)".into(),
                        required: false,
                        default: Some(serde_json::json!(false)),
                    },
                ],
            },
            Arc::new(TypeText { state: Arc::clone(&state) }),
        ),
        (
            ToolDef {
                name: "press_shortcut".into(),
                description: "Press a keyboard shortcut combination. \
                    Requires PIN confirmation (RED tier). \
                    Dangerous system combinations are logged.".into(),
                category: "gui_automation".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    ParamDef {
                        name: "keys".into(),
                        param_type: "array".into(),
                        description: "Array of keys to press (e.g., [\"ctrl\", \"s\"])".into(),
                        required: true,
                        default: None,
                    },
                    ParamDef {
                        name: "hold_duration_ms".into(),
                        param_type: "integer".into(),
                        description: "Duration to hold keys in milliseconds (optional)".into(),
                        required: false,
                        default: None,
                    },
                ],
            },
            Arc::new(PressShortcut { state: Arc::clone(&state) }),
        ),
        (
            ToolDef {
                name: "release_all".into(),
                description: "Release all keyboard modifiers (Shift, Control, Alt, Super) to clear stuck keys. \
                    GREEN tier: harmless modifier release for pre-input sanitization.".into(),
                category: "gui_automation".into(),
                default_tier: RiskLevel::Green,
                min_tier: "standard",
                parameters: vec![],
            },
            Arc::new(ReleaseAll { state: Arc::clone(&state) }),
        ),
        (
            ToolDef {
                name: "focus_window".into(),
                description: "Activate the current active window to ensure keyboard focus (X11). \
                    GREEN tier: harmless window activation for X11 focus.".into(),
                category: "gui_automation".into(),
                default_tier: RiskLevel::Green,
                min_tier: "standard",
                parameters: vec![],
            },
            Arc::new(FocusWindow { state: Arc::clone(&state) }),
        ),
        (
            ToolDef {
                name: "system_sleep".into(),
                description: "Sleep for a specified duration to allow UI to stabilize. \
                    GREEN tier: harmless wait with no side effects.".into(),
                category: "gui_automation".into(),
                default_tier: RiskLevel::Green,
                min_tier: "standard",
                parameters: vec![
                    ParamDef {
                        name: "duration_ms".into(),
                        param_type: "integer".into(),
                        description: "Duration to sleep in milliseconds".into(),
                        required: true,
                        default: None,
                    },
                ],
            },
            Arc::new(SystemSleep),
        ),
    ];

    let tool_count = tools.len();
    for (def, handler) in tools {
        reg.register(def, handler);
    }

    tracing::info!(target: "gui_automation", "Registered {} GUI automation tools", tool_count);
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protected_mode_detection() {
        let detector = ProtectedModeDetector::new();

        // Test password manager detection
        let keepass = WindowInfo {
            title: "KeePass - Password Database".to_string(),
            class: "keepass".to_string(),
            pid: 1234,
        };
        assert!(detector.is_protected(&keepass));

        // Test safe window
        let safe = WindowInfo {
            title: "Document - LibreOffice Writer".to_string(),
            class: "libreoffice".to_string(),
            pid: 5678,
        };
        assert!(!detector.is_protected(&safe));
    }

    #[test]
    fn test_key_parsing() {
        assert!(matches!(parse_key_string("ctrl").unwrap(), Key::Control));
        assert!(matches!(parse_key_string("s").unwrap(), Key::Char('s')));
        assert!(matches!(parse_key_string("F12").unwrap(), Key::F12));
        assert!(matches!(parse_key_string("Escape").unwrap(), Key::Escape));
    }

    #[test]
    fn test_control_char_detection() {
        let has_control = "rm -rf /".chars().any(|c| SHELL_CONTROL_CHARS.contains(&c));
        assert!(!has_control); // Space is not in the list

        let has_pipe = "cat file | grep text"
            .chars()
            .any(|c| SHELL_CONTROL_CHARS.contains(&c));
        assert!(has_pipe);
    }
}
