//! KRIA Wake Daemon (Voice System v3, Wave 9).
//!
//! An OPTIONAL, unprivileged, always-on helper that runs ONLY voice-activity
//! detection + wake-word detection. It contains NO STT, TTS, or LLM — on a
//! wake-phrase hit it signals the main KRIA app over a Unix socket so the app
//! can launch/focus and start a real voice session (Requirement 11).
//!
//! Safety/behaviour:
//! - Runs as the normal user (no privilege escalation).
//! - Holds the microphone only while enabled; logs a visible mic-active banner
//!   so the user always knows it is listening (Requirement 11.3).
//! - If the main app is not listening on the socket, it optionally launches the
//!   app via `KRIA_WAKE_LAUNCH`; otherwise it logs and keeps waiting. In-app
//!   wake remains the fallback (Requirement 11.4).
//!
//! Configuration (env):
//! - `KRIA_WAKE_MODEL`     — path to the openWakeWord keyword `.onnx`
//!                           (default: discovered `models/wake/hey_ria.onnx`).
//! - `KRIA_WAKE_SENSITIVITY` — 0.0–1.0 (default 0.5).
//! - `KRIA_WAKE_SOCK`      — app IPC socket path (see `ipc::resolve_socket_path`).
//! - `KRIA_WAKE_LAUNCH`    — command to launch the app when no listener present.
//! - `KRIA_WAKE_MIC`       — input device name (default: system default).

mod ipc;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

use ipc::{resolve_socket_path, WakeSignal};
use kria_core::voice::capture::{AudioCapture, AudioChunk};
use kria_core::voice::v2::WakeWordDetector;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Discover `models/wake/hey_ria.onnx` by walking up from the exe / CWD.
fn discover_wake_model() -> PathBuf {
    if let Ok(p) = std::env::var("KRIA_WAKE_MODEL") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        roots.push(exe);
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    for start in roots {
        let mut dir = Some(start.as_path());
        while let Some(d) = dir {
            let candidate = d.join("models/wake/hey_ria.onnx");
            if candidate.exists() {
                return candidate;
            }
            dir = d.parent();
        }
    }
    PathBuf::from("models/wake/hey_ria.onnx")
}

/// Send a wake signal to the app. Returns `true` when delivered to a listener.
async fn send_wake_signal(socket: &PathBuf, sig: &WakeSignal) -> bool {
    match tokio::time::timeout(Duration::from_millis(500), UnixStream::connect(socket)).await {
        Ok(Ok(mut stream)) => {
            if stream.write_all(&sig.encode_line()).await.is_ok() {
                let _ = stream.flush().await;
                tracing::info!(?socket, "wake signal delivered to app");
                return true;
            }
            false
        }
        _ => false,
    }
}

/// Best-effort app launch when no listener is present.
fn try_launch_app() {
    if let Ok(cmd) = std::env::var("KRIA_WAKE_LAUNCH") {
        let cmd = cmd.trim().to_string();
        if !cmd.is_empty() {
            tracing::info!(%cmd, "no app listener — launching app via KRIA_WAKE_LAUNCH");
            // Split on whitespace; run detached.
            let mut parts = cmd.split_whitespace();
            if let Some(program) = parts.next() {
                let args: Vec<&str> = parts.collect();
                let _ = std::process::Command::new(program).args(args).spawn();
            }
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let model = discover_wake_model();
    let sensitivity: f32 = std::env::var("KRIA_WAKE_SENSITIVITY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.5);
    let socket = resolve_socket_path();
    let mic = std::env::var("KRIA_WAKE_MIC").unwrap_or_else(|_| "auto".to_string());

    let detector = WakeWordDetector::try_load(
        model.clone(),
        sensitivity,
        "hey ria",
        vec![
            "hey ria".into(),
            "hey riya".into(),
            "hello ria".into(),
            "hello riya".into(),
        ],
    );
    if !detector.is_active() {
        anyhow::bail!(
            "wake detector inactive (model missing/unloadable at {} or feature disabled); \
             daemon exiting — in-app wake remains available",
            model.display()
        );
    }

    // Requirement 11.3: make microphone use visible to the user.
    tracing::warn!(
        model = %model.display(),
        sensitivity,
        ?socket,
        "🎙️  KRIA wake daemon ACTIVE — microphone is being monitored for the wake phrase only (no recording/STT/LLM)."
    );

    // Audio capture → broadcast → wake detector.
    let capture = AudioCapture::new(16_000)
        .with_input_device(mic)
        .follow_system_default(true);
    let (mut capture_rx, _handle) = capture
        .start()
        .map_err(|e| anyhow::anyhow!("failed to start microphone capture: {e}"))?;

    let (bcast_tx, _) = tokio::sync::broadcast::channel::<AudioChunk>(128);
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
    Arc::new(detector).spawn(bcast_tx.subscribe(), wake_tx);

    // Forward captured chunks into the broadcast the detector subscribes to.
    let bcast_for_fwd = bcast_tx.clone();
    tokio::spawn(async move {
        while let Some(chunk) = capture_rx.recv().await {
            let _ = bcast_for_fwd.send(chunk);
        }
        tracing::warn!("wake daemon: capture stream ended");
    });

    // Debounce repeated fires within a short window.
    let mut last_fire_ms = 0u64;
    while let Some(ev) = wake_rx.recv().await {
        let ts = now_ms();
        if ts.saturating_sub(last_fire_ms) < 1500 {
            continue;
        }
        last_fire_ms = ts;
        tracing::info!(score = ev.score, source = %ev.source, "wake phrase detected");
        let sig = WakeSignal::Wake {
            score: ev.score,
            source: ev.source.clone(),
            ts_ms: ts,
        };
        if !send_wake_signal(&socket, &sig).await {
            tracing::info!("no app listener on wake socket");
            try_launch_app();
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    if let Err(e) = run().await {
        tracing::error!(error = %e, "wake daemon exited");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn wake_signal_delivered_to_listener() {
        let dir = std::env::temp_dir();
        let sock = dir.join(format!("kria-wake-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap();

        // App-side: accept one connection and read the framed signal.
        let sock_for_accept = sock.clone();
        let accept = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.unwrap();
            let _ = std::fs::remove_file(&sock_for_accept);
            let line = buf.split(|b| *b == b'\n').next().unwrap().to_vec();
            WakeSignal::decode_line(&line).unwrap()
        });

        let sig = WakeSignal::Wake {
            score: 0.9,
            source: "oww".into(),
            ts_ms: now_ms(),
        };
        let delivered = send_wake_signal(&sock, &sig).await;
        assert!(delivered, "signal should be delivered to the listener");

        let received = accept.await.unwrap();
        assert_eq!(received, sig);
    }

    #[tokio::test]
    async fn send_returns_false_when_no_listener() {
        let sock = std::env::temp_dir().join("kria-wake-nolistener.sock");
        let _ = std::fs::remove_file(&sock);
        let sig = WakeSignal::Ping { ts_ms: 1 };
        assert!(!send_wake_signal(&sock, &sig).await);
    }
}
