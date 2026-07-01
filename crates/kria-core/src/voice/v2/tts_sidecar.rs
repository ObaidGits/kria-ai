//! Kokoro TTS sidecar launcher + liveness (Voice System v3, Wave 5).
//!
//! Mirrors `stt_sidecar`: resolves the base URL, probes `/health`, and
//! best-effort spawns `sidecars/kria-tts/main.py`. The Rust `KokoroTts` engine
//! uses this; when the sidecar/model is unavailable it falls back to Piper
//! (Requirement 7.4 — guaranteed fallback engine).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Default sidecar bind URL (matches `KRIA_TTS_PORT` default in main.py).
pub const DEFAULT_TTS_SIDECAR_URL: &str = "http://127.0.0.1:8766";

static SIDECAR_CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

fn child_slot() -> &'static Mutex<Option<Child>> {
    SIDECAR_CHILD.get_or_init(|| Mutex::new(None))
}

/// Resolve the sidecar base URL: `KRIA_TTS_SIDECAR_URL` env or the default.
pub fn base_url() -> String {
    std::env::var("KRIA_TTS_SIDECAR_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_TTS_SIDECAR_URL.to_string())
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

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut cur = Some(start);
    while let Some(dir) = cur {
        if dir.join("sidecars/kria-tts/main.py").exists() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

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

fn resolve_python(root: &Path) -> String {
    if let Ok(p) = std::env::var("KRIA_TTS_PYTHON") {
        if !p.trim().is_empty() {
            return p;
        }
    }
    // Prefer the DEDICATED sidecar venv first: Kokoro's deps (kokoro/misaki/
    // spacy/torch) live here on a compatible Python (3.10–3.12). The repo
    // `.venv` is often a different Python (e.g. 3.14) without these wheels, so
    // it must NOT take precedence for the TTS sidecar.
    let sidecar_venv = root.join("sidecars/kria-tts/venv/bin/python");
    if sidecar_venv.exists() {
        return sidecar_venv.to_string_lossy().into_owned();
    }
    let venv = root.join(".venv/bin/python");
    if venv.exists() {
        return venv.to_string_lossy().into_owned();
    }
    "python3".to_string()
}

fn parse_host_port(base: &str) -> (String, u16) {
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
        .unwrap_or(8766);
    (host, port)
}

async fn spawn_if_needed(base: &str) -> bool {
    let mut slot = child_slot().lock().await;
    if let Some(child) = slot.as_mut() {
        // If a previously-spawned child is still alive, reuse it. If it has
        // exited (e.g. wrong-Python import failure, crash), clear the slot and
        // respawn so the sidecar self-heals.
        match child.try_wait() {
            Ok(Some(status)) => {
                tracing::warn!(?status, "tts sidecar: previous child exited; respawning");
                *slot = None;
            }
            _ => return true, // still running
        }
    }
    let Some(root) = workspace_root() else {
        tracing::warn!("tts sidecar: could not locate workspace root (sidecars/kria-tts/main.py)");
        return false;
    };
    let script = root.join("sidecars/kria-tts/main.py");
    let python = resolve_python(&root);
    let (host, port) = parse_host_port(base);

    let mut cmd = Command::new(&python);
    cmd.arg(&script)
        .current_dir(&root)
        .env("KRIA_TTS_HOST", host)
        .env("KRIA_TTS_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for key in ["KRIA_TTS_LANG", "KRIA_TTS_VOICE"] {
        if let Ok(v) = std::env::var(key) {
            cmd.env(key, v);
        }
    }

    match cmd.spawn() {
        Ok(child) => {
            tracing::info!(python = %python, script = %script.display(), "tts sidecar: spawned Kokoro process");
            *slot = Some(child);
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, python = %python, "tts sidecar: spawn failed");
            false
        }
    }
}

/// Ensure a healthy Kokoro sidecar is reachable. Spawns once and polls
/// `/health` until the model loads or `max_wait` elapses. Returns readiness.
pub async fn ensure_ready(client: &reqwest::Client, base: &str, max_wait: Duration) -> bool {
    if is_healthy(client, base).await {
        return true;
    }
    if !spawn_if_needed(base).await {
        return is_healthy(client, base).await;
    }
    let deadline = std::time::Instant::now() + max_wait;
    while std::time::Instant::now() < deadline {
        if is_healthy(client, base).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    tracing::warn!(base, "tts sidecar: not healthy within {:?}", max_wait);
    false
}

/// Warm the Kokoro sidecar at voice-session start so the first spoken sentence
/// does not pay the model cold-load cost. Best-effort; non-blocking for the
/// caller (run in a task). No-op cost when Kokoro isn't selected/installed.
pub async fn warm_up() {
    let client = reqwest::Client::new();
    let base = base_url();
    if ensure_ready(&client, &base, Duration::from_secs(30)).await {
        tracing::info!(base, "tts sidecar (kokoro): warm and ready");
    } else {
        tracing::warn!(
            base,
            "tts sidecar (kokoro): warm-up incomplete (will fall back to Piper)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_defaults() {
        std::env::remove_var("KRIA_TTS_SIDECAR_URL");
        assert_eq!(base_url(), DEFAULT_TTS_SIDECAR_URL);
    }

    #[test]
    fn parses_host_and_port() {
        assert_eq!(
            parse_host_port("http://127.0.0.1:8766"),
            ("127.0.0.1".into(), 8766)
        );
        assert_eq!(
            parse_host_port("http://localhost:9100/"),
            ("localhost".into(), 9100)
        );
    }
}
