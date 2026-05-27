/// Linux implementation of `OsIntentBackend`.
///
/// # URI dispatch
/// Uses a multi-strategy opener that works on both X11 and Wayland:
/// 1. `xdg-open` — standard freedesktop handler (works on both X11 and Wayland)
/// 2. `gio open` — GNOME fallback (Wayland-native)
/// 3. `kde-open` / `kde-open5` — KDE fallback
/// 4. `open` crate — last resort
///
/// # App launch
/// Uses `gio launch <.desktop-path>` so that `.desktop` `Exec=` field-codes
/// (`%U`, `%f`, `%F`) are handled correctly by gio's own parser — no naive
/// string substitution is performed in K.R.I.A. code.
///
/// # AX (accessibility)
/// Stub returning `Err("not yet implemented")` — AT-SPI implementation is deferred
/// to Phase E+1 as the three target use cases are 100% URI-resolvable.
use std::collections::HashSet;
use std::path::Path;

use async_trait::async_trait;
use tracing::{info, warn};

use super::OsIntentBackend;
use crate::platform::app_registry::InstalledAppRegistry;
use crate::platform::intent::capability::{AxAction, CanonicalAppId, SafeArg};

pub struct LinuxBackend {
    registry: std::sync::Arc<InstalledAppRegistry>,
}

impl LinuxBackend {
    pub fn new(registry: std::sync::Arc<InstalledAppRegistry>) -> Self {
        Self { registry }
    }
}

fn first_existing_command(candidates: &[&str]) -> String {
    for candidate in candidates {
        if candidate.starts_with('/') && Path::new(candidate).exists() {
            return (*candidate).to_string();
        }
    }
    candidates.last().copied().unwrap_or("").to_string()
}

fn trusted_gio_command() -> String {
    first_existing_command(&["/usr/bin/gio", "/bin/gio", "gio"])
}

fn direct_code_command(app_id: &CanonicalAppId) -> Option<String> {
    match app_id.as_str().to_ascii_lowercase().as_str() {
        "code" | "vscode" | "visual-studio-code" => Some(first_existing_command(&[
            "/usr/bin/code",
            "/usr/local/bin/code",
            "code",
        ])),
        "code-oss" => Some(first_existing_command(&[
            "/usr/bin/code-oss",
            "/usr/local/bin/code-oss",
            "code-oss",
        ])),
        "vscodium" => Some(first_existing_command(&[
            "/usr/bin/codium",
            "/usr/local/bin/codium",
            "codium",
        ])),
        _ => None,
    }
}

/// Detect the current display server session type.
/// Returns "wayland", "x11", or "unknown".
/// Cached via OnceLock — env vars are read once at first call.
fn detect_session_type() -> &'static str {
    use std::sync::OnceLock;
    static SESSION_TYPE: OnceLock<&'static str> = OnceLock::new();
    SESSION_TYPE.get_or_init(|| {
        // XDG_SESSION_TYPE is the most reliable indicator set by the display manager.
        if let Ok(session) = std::env::var("XDG_SESSION_TYPE") {
            let lower = session.to_ascii_lowercase();
            if lower.contains("wayland") {
                return "wayland";
            }
            if lower.contains("x11") || lower.contains("mir") {
                return "x11";
            }
        }
        // WAYLAND_DISPLAY is set when a Wayland compositor is running.
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            return "wayland";
        }
        // DISPLAY is set for X11 sessions.
        if std::env::var("DISPLAY").is_ok() {
            return "x11";
        }
        "unknown"
    })
}

/// Try to open a URI using a specific command.
/// Returns Ok(()) if the command spawned successfully (not necessarily if the URL opened).
async fn try_open_with(cmd: &str, args: &[&str], url: &str) -> Result<(), String> {
    let mut command = tokio::process::Command::new(cmd);
    command.args(args).arg(url);

    // Inherit the current environment so WAYLAND_DISPLAY / DISPLAY are available.
    // This is critical for Wayland — xdg-open needs WAYLAND_DISPLAY to find the compositor.
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{cmd} failed: {e}"))
}

#[async_trait]
impl OsIntentBackend for LinuxBackend {
    async fn open_uri(&self, url: &url::Url) -> Result<(), String> {
        let url_str = url.as_str();
        let session = detect_session_type();
        info!(url = url_str, session, "LinuxBackend::open_uri");

        // Strategy order depends on session type.
        // On Wayland: prefer gio open (native) → xdg-open → kde-open → open crate
        // On X11: prefer xdg-open → gio open → kde-open → open crate
        // On unknown: try all in order

        let strategies: &[(&str, &[&str])] = match session {
            "wayland" => &[
                ("xdg-open", &[] as &[&str]), // xdg-open works on Wayland via portal
                ("gio", &["open"]),           // GNOME Wayland-native
                ("kde-open5", &[]),           // KDE Plasma 5
                ("kde-open", &[]),            // KDE Plasma 4 fallback
            ],
            "x11" => &[
                ("xdg-open", &[]),
                ("gio", &["open"]),
                ("kde-open5", &[]),
                ("kde-open", &[]),
            ],
            _ => &[
                ("xdg-open", &[]),
                ("gio", &["open"]),
                ("kde-open5", &[]),
                ("kde-open", &[]),
            ],
        };

        let mut last_error = String::new();
        for (cmd, args) in strategies {
            match try_open_with(cmd, args, url_str).await {
                Ok(()) => {
                    info!(cmd, url = url_str, "LinuxBackend::open_uri succeeded");
                    return Ok(());
                }
                Err(e) => {
                    // Command not found or failed to spawn — try next strategy
                    last_error = e;
                }
            }
        }

        // Final fallback: the `open` crate (handles edge cases)
        match open::that_detached(url_str) {
            Ok(()) => {
                info!(
                    url = url_str,
                    "LinuxBackend::open_uri succeeded via open crate"
                );
                Ok(())
            }
            Err(e) => {
                warn!(
                    url = url_str,
                    session,
                    last_strategy_error = %last_error,
                    "LinuxBackend::open_uri: all strategies failed"
                );
                Err(format!(
                    "open_uri failed on {session} session: {e}. \
                    Tried: xdg-open, gio open, kde-open. \
                    Ensure xdg-utils is installed and a default browser is configured."
                ))
            }
        }
    }

    async fn launch_app(&self, app_id: &CanonicalAppId, args: &[SafeArg]) -> Result<u32, String> {
        info!(app_id = app_id.as_str(), "LinuxBackend::launch_app");

        if let Some(code_cmd) = direct_code_command(app_id) {
            let mut cmd = tokio::process::Command::new(&code_cmd);
            cmd.arg("--reuse-window");
            for arg in args {
                cmd.arg(arg.as_str());
            }
            match cmd.spawn() {
                Ok(child) => {
                    let pid = child.id().unwrap_or(0);
                    info!(
                        app_id = app_id.as_str(),
                        command = %code_cmd,
                        pid,
                        "LinuxBackend::launch_app succeeded via direct VS Code command"
                    );
                    return Ok(pid);
                }
                Err(error) => {
                    warn!(
                        app_id = app_id.as_str(),
                        command = %code_cmd,
                        error = %error,
                        "LinuxBackend::launch_app direct VS Code command failed; falling back to desktop launcher"
                    );
                }
            }
        }

        // Look up the .desktop path from the registry.
        let desktop_path = self
            .registry
            .desktop_path(app_id)
            .ok_or_else(|| format!("no .desktop file found for '{}'", app_id.as_str()))?;

        // `gio launch <path>` handles Exec= field-code substitution (%U, %f, %F etc.)
        // using gio's own parser — safe from injection via our SafeArg tokens.
        //
        // NOTE: We pass SafeArg values as additional arguments ONLY if gio launch supports
        // them for the specific Exec= entry type. For %U-type entries, gio appends the args
        // as URLs; for plain Exec= entries they are positional.
        let gio = trusted_gio_command();
        let mut cmd = tokio::process::Command::new(&gio);
        cmd.arg("launch").arg(&desktop_path);
        for arg in args {
            cmd.arg(arg.as_str());
        }

        let child = cmd
            .spawn()
            .map_err(|e| format!("{gio} launch failed for '{}': {e}", app_id.as_str()))?;

        let pid = child.id().unwrap_or(0);
        info!(
            app_id = app_id.as_str(),
            command = %gio,
            pid,
            "LinuxBackend::launch_app succeeded via desktop launcher"
        );
        Ok(pid)
    }

    async fn ax_invoke(&self, app_id: &CanonicalAppId, _action: &AxAction) -> Result<(), String> {
        // AT-SPI implementation deferred to Phase E+1.
        // The three initial use cases (Chrome search, WhatsApp, YouTube) are fully
        // URI-resolvable and do not require accessibility automation.
        warn!(
            app_id = app_id.as_str(),
            "ax_invoke not yet implemented on Linux (AT-SPI deferred)"
        );
        Err(format!(
            "accessibility automation for '{}' is not yet implemented on Linux; \
             use URI deep-links instead",
            app_id.as_str()
        ))
    }

    fn registered_schemes(&self) -> HashSet<String> {
        self.registry.registered_schemes()
    }
}
