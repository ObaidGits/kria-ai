//! App-side wake-daemon listener (Voice System v3, Wave 9 warm path).
//!
//! Binds the wake IPC socket and waits for the optional `kria-wake-daemon` to
//! deliver a `WakeSignal`. On a wake, it emits a `voice:external_wake` Tauri
//! event so the frontend can start a voice session (the app may already be
//! running, so no relaunch is needed — this is the warm auto-start path that
//! complements the daemon's cold-start `KRIA_WAKE_LAUNCH`).
//!
//! Unprivileged: the app owns the socket; the daemon merely connects. If the
//! daemon is never started, this listener simply idles. In-app wake remains the
//! fallback regardless (Requirement 11.4).

use std::path::PathBuf;

use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixListener;

/// Parsed wake signal (mirrors `kria-wake-daemon::ipc::WakeSignal`). Kept local
/// so the desktop crate does not depend on the daemon binary.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WakeSignal {
    Wake {
        score: f32,
        source: String,
        ts_ms: u64,
    },
    Ping {
        ts_ms: u64,
    },
}

/// Resolve the wake-daemon socket path (must match the daemon's resolver).
pub fn resolve_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("KRIA_WAKE_SOCK") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let mut path = PathBuf::from(xdg);
        path.push("kria");
        path.push("wake.sock");
        return path;
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/kria-wake-{uid}.sock"))
}

/// Parse one newline-delimited JSON wake line. Pure (unit-tested).
pub fn parse_signal_line(line: &str) -> Option<WakeSignal> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    serde_json::from_str::<WakeSignal>(line).ok()
}

/// Spawn the listener. Best-effort: binds the socket (unlinking any stale one),
/// accepts daemon connections, and emits `voice:external_wake` on each wake.
/// Non-fatal on bind failure (logs + returns).
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let socket = resolve_socket_path();
        if let Some(parent) = socket.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        // Unlink stale socket so bind succeeds across restarts.
        let _ = tokio::fs::remove_file(&socket).await;

        let listener = match UnixListener::bind(&socket) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, ?socket, "wake listener: bind failed (wake daemon warm-path disabled)");
                return;
            }
        };
        tracing::info!(
            ?socket,
            "wake listener: bound — awaiting optional wake daemon"
        );

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match reader.read_line(&mut line).await {
                            Ok(0) => break, // connection closed
                            Ok(_) => {
                                if let Some(WakeSignal::Wake { score, source, .. }) =
                                    parse_signal_line(&line)
                                {
                                    tracing::info!(score, %source, "wake listener: external wake received");
                                    let _ = app.emit(
                                        "voice:external_wake",
                                        serde_json::json!({ "score": score, "source": source }),
                                    );
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "wake listener: accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wake_and_ping() {
        let w =
            parse_signal_line("{\"type\":\"wake\",\"score\":0.8,\"source\":\"oww\",\"ts_ms\":5}");
        assert_eq!(
            w,
            Some(WakeSignal::Wake {
                score: 0.8,
                source: "oww".into(),
                ts_ms: 5
            })
        );
        let p = parse_signal_line("{\"type\":\"ping\",\"ts_ms\":1}\n");
        assert_eq!(p, Some(WakeSignal::Ping { ts_ms: 1 }));
    }

    #[test]
    fn rejects_garbage_and_empty() {
        assert_eq!(parse_signal_line(""), None);
        assert_eq!(parse_signal_line("not json"), None);
        assert_eq!(parse_signal_line("{\"type\":\"unknown\"}"), None);
    }

    #[test]
    fn socket_path_env_override() {
        std::env::set_var("KRIA_WAKE_SOCK", "/tmp/app-side-wake.sock");
        assert_eq!(
            resolve_socket_path(),
            PathBuf::from("/tmp/app-side-wake.sock")
        );
        std::env::remove_var("KRIA_WAKE_SOCK");
    }
}
