//! faster-whisper STT sidecar launcher + liveness (Voice System v3, Wave A/A3).
//!
//! Resolves the sidecar base URL, probes `/health`, and (best-effort) spawns
//! the Python sidecar (`sidecars/kria-stt/main.py`) when it is not already
//! running. The child is held in a process-global guard with kill-on-drop so a
//! single warm instance is shared for the app lifetime (load once, keep warm —
//! Wave A3).
//!
//! Liveness is the foundation of the no-hang guarantee (Requirement 6.5): the
//! client only streams audio after `/health` reports `model_loaded`, and the
//! pipeline watchdog (Wave 1) bounds the transcribe call regardless.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Default sidecar bind URL (matches `KRIA_STT_PORT` default in main.py).
pub const DEFAULT_STT_SIDECAR_URL: &str = "http://127.0.0.1:8765";

/// Holds the spawned sidecar child so it is killed on app exit (kill_on_drop).
/// `OnceLock<Mutex<Option<Child>>>` — initialised on first spawn attempt.
static SIDECAR_CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

fn child_slot() -> &'static Mutex<Option<Child>> {
    SIDECAR_CHILD.get_or_init(|| Mutex::new(None))
}

/// Resolve the sidecar base URL: `KRIA_STT_SIDECAR_URL` env or the default.
pub fn base_url() -> String {
    std::env::var("KRIA_STT_SIDECAR_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_STT_SIDECAR_URL.to_string())
}

/// True when the sidecar's `/health` reports a loaded model.
pub async fn is_healthy(client: &reqwest::Client, base: &str) -> bool {
    let url = format!("{}/health", base.trim_end_matches('/'));
    match client
        .get(&url)
        .timeout(Duration::from_millis(800))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(v) => v
                .get("model_loaded")
                .and_then(|b| b.as_bool())
                .unwrap_or(false),
            Err(_) => false,
        },
        _ => false,
    }
}

/// Walk up from `start` looking for `sidecars/kria-stt/main.py`.
fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut cur = Some(start);
    while let Some(dir) = cur {
        if dir.join("sidecars/kria-stt/main.py").exists() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

/// Discover the workspace root via the current exe path, then the CWD.
fn workspace_root() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = find_workspace_root(&exe) {
            return Some(root);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(root) = find_workspace_root(&cwd) {
            return Some(root);
        }
    }
    None
}

/// Pick the Python interpreter for the sidecar.
///
/// Priority: `KRIA_STT_PYTHON` → workspace `.venv/bin/python` → sidecar-local
/// `venv/bin/python` → `python3`.
fn resolve_python(root: &Path) -> String {
    if let Ok(p) = std::env::var("KRIA_STT_PYTHON") {
        if !p.trim().is_empty() {
            return p;
        }
    }
    let venv = root.join(".venv/bin/python");
    if venv.exists() {
        return venv.to_string_lossy().into_owned();
    }
    let sidecar_venv = root.join("sidecars/kria-stt/venv/bin/python");
    if sidecar_venv.exists() {
        return sidecar_venv.to_string_lossy().into_owned();
    }
    "python3".to_string()
}

/// Best-effort spawn of the sidecar process (idempotent — only spawns when the
/// global slot is empty). Returns `false` when the script/python cannot be
/// resolved or the spawn fails; the caller then degrades to the CLI fallback.
async fn spawn_if_needed(base: &str) -> bool {
    let mut slot = child_slot().lock().await;
    if let Some(child) = slot.as_mut() {
        // Reuse a live child; respawn if the previous one exited (self-heal).
        match child.try_wait() {
            Ok(Some(status)) => {
                tracing::warn!(?status, "stt sidecar: previous child exited; respawning");
                *slot = None;
            }
            _ => return true,
        }
    }

    let Some(root) = workspace_root() else {
        tracing::warn!("stt sidecar: could not locate workspace root (sidecars/kria-stt/main.py)");
        return false;
    };
    let script = root.join("sidecars/kria-stt/main.py");
    let python = resolve_python(&root);

    // Parse host/port out of the base URL so a custom KRIA_STT_SIDECAR_URL is
    // honoured by the spawned process too.
    let (host, port) = parse_host_port(base);

    let mut cmd = Command::new(&python);
    cmd.arg(&script)
        .current_dir(&root)
        .env("KRIA_STT_HOST", host)
        .env("KRIA_STT_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // Pass through model/device tuning if the operator set them.
    for key in [
        "KRIA_STT_MODEL",
        "KRIA_STT_DEVICE",
        "KRIA_STT_COMPUTE",
        "KRIA_STT_MIN_FREE_VRAM",
    ] {
        if let Ok(v) = std::env::var(key) {
            cmd.env(key, v);
        }
    }

    match cmd.spawn() {
        Ok(child) => {
            tracing::info!(
                python = %python,
                script = %script.display(),
                "stt sidecar: spawned faster-whisper process"
            );
            *slot = Some(child);
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, python = %python, "stt sidecar: spawn failed");
            false
        }
    }
}

fn parse_host_port(base: &str) -> (String, u16) {
    // Minimal parse: strip scheme, split host:port.
    let no_scheme = base
        .trim()
        .trim_end_matches('/')
        .splitn(2, "://")
        .last()
        .unwrap_or(base);
    let mut parts = no_scheme.splitn(2, ':');
    let host = parts.next().unwrap_or("127.0.0.1").to_string();
    let port = parts
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8765);
    (host, port)
}

/// Ensure a healthy sidecar is reachable at `base`. If already healthy, returns
/// immediately. Otherwise spawns the process (once) and polls `/health` until
/// the model loads or `max_wait` elapses.
///
/// Returns `true` when the sidecar is healthy and ready to transcribe.
pub async fn ensure_ready(client: &reqwest::Client, base: &str, max_wait: Duration) -> bool {
    if is_healthy(client, base).await {
        return true;
    }
    if !spawn_if_needed(base).await {
        // No process to wait on; one more probe in case an external sidecar is
        // mid-startup, then give up so the caller falls back.
        return is_healthy(client, base).await;
    }
    let deadline = std::time::Instant::now() + max_wait;
    while std::time::Instant::now() < deadline {
        if is_healthy(client, base).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    tracing::warn!(base, "stt sidecar: not healthy within {:?}", max_wait);
    false
}

/// Warm the sidecar at voice-session start (Wave A3.2): spawns/loads the model
/// up front so the FIRST utterance does not pay the multi-second cold-load
/// cost. Best-effort and non-blocking for the caller (run it in a task).
pub async fn warm_up() {
    let client = reqwest::Client::new();
    let base = base_url();
    let ready = ensure_ready(&client, &base, Duration::from_secs(30)).await;
    if ready {
        tracing::info!(base, "stt sidecar: warm and ready");
    } else {
        tracing::warn!(base, "stt sidecar: warm-up did not complete (will retry on first utterance / fall back to CLI)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_defaults() {
        // Without the env var set, the default is returned.
        std::env::remove_var("KRIA_STT_SIDECAR_URL");
        assert_eq!(base_url(), DEFAULT_STT_SIDECAR_URL);
    }

    #[test]
    fn parses_host_and_port() {
        assert_eq!(
            parse_host_port("http://127.0.0.1:8765"),
            ("127.0.0.1".into(), 8765)
        );
        assert_eq!(
            parse_host_port("http://localhost:9000/"),
            ("localhost".into(), 9000)
        );
        assert_eq!(
            parse_host_port("127.0.0.1:1234"),
            ("127.0.0.1".into(), 1234)
        );
    }
}
