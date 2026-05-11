//! ComfyUI WebSocket progress bridge.
//!
//! Spawns a dedicated task that:
//! 1. Connects to `ws://127.0.0.1:{port}/ws?clientId={client_id}`
//! 2. Parses Comfy progress/status/executing/executed frames
//! 3. Fans them out to the Tauri app handle via `app.emit()`
//! 4. Resolves a `oneshot` sender when the final "executed" frame arrives
//!
//! Heartbeat: sends a `{"op":"ping"}` every 2 s; reconnects if no response
//! for a sustained quiet window. State is recovered via `GET /history/{prompt_id}`.

use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const WS_PING_INTERVAL_SECS: u64 = 2;
const WS_QUIET_HISTORY_FALLBACK_SECS: u64 = 25;

/// A type-erased event emitter — fulfilled by `Arc<tauri::AppHandle>` in the desktop
/// crate, or a no-op in tests / server builds.
pub type EventEmitter = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// Output path of a completed ComfyUI job.
#[derive(Debug, Clone)]
pub struct ComfyOutput {
    pub filename: String,
    pub subfolder: String,
    pub output_type: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WsBridgeError {
    #[error("WebSocket connection failed: {0}")]
    Connect(String),
    #[error("Job timed out after {seconds}s")]
    Timeout { seconds: u64 },
    #[error("HTTP error (status: {status:?}): {message}")]
    Http {
        status: Option<u16>,
        message: String,
    },
    #[error("ComfyUI reported an error: {message}")]
    ComfyError { message: String },
    #[error("Cancelled")]
    Cancelled,
}

/// Spawn a WebSocket listener for a single ComfyUI job.
///
/// Returns a handle to the spawned task. Drop the handle to cancel
/// and interrupt ComfyUI backend execution.
pub fn spawn_ws_listener(
    port: u16,
    client_id: String,
    prompt_id: String,
    emitter: Option<EventEmitter>,
    completion_tx: oneshot::Sender<Result<Vec<ComfyOutput>, WsBridgeError>>,
    cancel: tokio_util::sync::CancellationToken,
    generation_timeout: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let url = format!("ws://127.0.0.1:{}/ws?clientId={}", port, client_id);
        let deadline = tokio::time::Instant::now() + generation_timeout;
        let mut outputs: Vec<ComfyOutput> = Vec::new();
        let mut last_pong = tokio::time::Instant::now();
        let mut last_connect_error: Option<String> = None;
        let mut terminal_error: Option<String> = None;

        // --- connect with retry ---
        let (ws_stream, _) = loop {
            if completion_tx.is_closed() {
                cancel.cancel();
                interrupt_comfy_backend(port, "completion receiver dropped before connect").await;
                return;
            }
            if cancel.is_cancelled() {
                interrupt_comfy_backend(port, "ws listener cancelled before connect").await;
                let _ = completion_tx.send(Err(WsBridgeError::Cancelled));
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                let message = last_connect_error.unwrap_or_else(|| {
                    "timed out before websocket connection was established".to_string()
                });
                let _ = completion_tx.send(Err(WsBridgeError::Connect(message)));
                return;
            }
            match connect_async(&url).await {
                Ok(pair) => break pair,
                Err(e) => {
                    last_connect_error = Some(e.to_string());
                    warn!(error = %e, "WsBridge: connect failed, retrying in 500ms");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        };

        let (mut write, mut read) = ws_stream.split();

        // Heartbeat task sends ping every 2 s.
        let cancel2 = cancel.clone();
        let ping_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(WS_PING_INTERVAL_SECS));
            loop {
                tokio::select! {
                    _ = cancel2.cancelled() => break,
                    _ = interval.tick() => {
                        let msg = Message::Text(r#"{"op":"ping"}"#.to_string().into());
                        if write.send(msg).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        // Main receive loop.
        loop {
            if completion_tx.is_closed() {
                ping_task.abort();
                cancel.cancel();
                interrupt_comfy_backend(port, "completion receiver dropped while streaming").await;
                return;
            }

            // Hard wall-clock deadline.
            if tokio::time::Instant::now() >= deadline {
                ping_task.abort();
                let _ = completion_tx.send(Err(WsBridgeError::Timeout {
                    seconds: generation_timeout.as_secs(),
                }));
                return;
            }

            // Quiet-stream check (switch to history polling after sustained silence).
            if last_pong.elapsed() > Duration::from_secs(WS_QUIET_HISTORY_FALLBACK_SECS) {
                info!(
                    quiet_secs = WS_QUIET_HISTORY_FALLBACK_SECS,
                    "WsBridge: websocket quiet; switching to /history recovery"
                );
                // Attempt recovery via /history.
                ping_task.abort();
                let recovered =
                    recover_from_history_with_deadline(port, &prompt_id, deadline).await;
                let _ = completion_tx.send(recovered);
                return;
            }

            // Poll with a short timeout so receiver-close cancellation can be
            // observed promptly even when ComfyUI is quiet.
            let next = tokio::time::timeout(Duration::from_millis(250), read.next()).await;
            match next {
                Ok(Some(Ok(msg))) => {
                    last_pong = tokio::time::Instant::now(); // any message counts as alive
                    match msg {
                        Message::Text(text) => {
                            handle_frame(
                                &text,
                                &prompt_id,
                                &emitter,
                                &mut outputs,
                                &completion_tx,
                                &ping_task,
                                &cancel,
                                &mut terminal_error,
                            )
                            .await;
                            // If cancelled (job done or error), we're done.
                            if cancel.is_cancelled() {
                                ping_task.abort();
                                if let Some(message) = terminal_error.take() {
                                    let _ = completion_tx
                                        .send(Err(WsBridgeError::ComfyError { message }));
                                } else if outputs.is_empty() {
                                    let _ = completion_tx.send(Err(WsBridgeError::ComfyError {
                                        message: "job cancelled or error".into(),
                                    }));
                                } else {
                                    let _ = completion_tx.send(Ok(outputs));
                                }
                                return;
                            }
                        }
                        Message::Close(_) => {
                            debug!("WsBridge: server closed WS, recovering from history");
                            ping_task.abort();
                            let recovered =
                                recover_from_history_with_deadline(port, &prompt_id, deadline)
                                    .await;
                            let _ = completion_tx.send(recovered);
                            return;
                        }
                        _ => {}
                    }
                }
                Err(_) => continue,
                Ok(Some(Err(e))) => {
                    warn!(error = %e, "WsBridge: read error");
                    ping_task.abort();
                    let recovered =
                        recover_from_history_with_deadline(port, &prompt_id, deadline).await;
                    let _ = completion_tx.send(recovered);
                    return;
                }
                Ok(None) => {
                    // Stream ended or deadline.
                    ping_task.abort();
                    let recovered =
                        recover_from_history_with_deadline(port, &prompt_id, deadline).await;
                    let _ = completion_tx.send(recovered);
                    return;
                }
            }

            if cancel.is_cancelled() {
                ping_task.abort();
                interrupt_comfy_backend(port, "ws listener cancelled").await;
                let _ = completion_tx.send(Err(WsBridgeError::Cancelled));
                return;
            }
        }
    })
}

async fn interrupt_comfy_backend(port: u16, reason: &str) {
    let url = format!("http://127.0.0.1:{}/interrupt", port);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    match client.post(&url).send().await {
        Ok(resp) => {
            debug!(
                reason,
                status = %resp.status(),
                "WsBridge: sent ComfyUI interrupt"
            );
        }
        Err(e) => {
            warn!(reason, error = %e, "WsBridge: failed to send ComfyUI interrupt");
        }
    }
}

// Inline async helper — avoids Box<dyn Future>.
async fn handle_frame(
    text: &str,
    prompt_id: &str,
    emitter: &Option<EventEmitter>,
    outputs: &mut Vec<ComfyOutput>,
    _tx: &oneshot::Sender<Result<Vec<ComfyOutput>, WsBridgeError>>,
    ping_task: &tokio::task::JoinHandle<()>,
    cancel: &tokio_util::sync::CancellationToken,
    terminal_error: &mut Option<String>,
) {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

    let emit = |event: &str, payload: serde_json::Value| {
        if let Some(e) = emitter {
            e(event, payload);
        }
    };

    match msg_type {
        "progress" => {
            let value = val["data"]["value"].as_u64().unwrap_or(0);
            let max = val["data"]["max"].as_u64().unwrap_or(1);
            let percent = (value * 100 / max.max(1)) as u32;
            emit(
                "image:progress",
                serde_json::json!({
                    "value": value,
                    "max": max,
                    "percent": percent,
                }),
            );
            debug!(value, max, "ComfyUI progress");
        }
        "executing" => {
            let node = val["data"]["node"].as_str().unwrap_or("?");
            emit("image:stage", serde_json::json!({ "node": node }));
        }
        "executed" => {
            // Only handle the frame for our prompt_id.
            if val["data"]["prompt_id"].as_str() != Some(prompt_id) {
                return;
            }
            // Collect output files.
            if let Some(images) = val["data"]["output"]["images"].as_array() {
                for img in images {
                    outputs.push(ComfyOutput {
                        filename: img["filename"].as_str().unwrap_or("").to_string(),
                        subfolder: img["subfolder"].as_str().unwrap_or("").to_string(),
                        output_type: img["type"].as_str().unwrap_or("output").to_string(),
                    });
                }
            }
            info!(
                outputs = outputs.len(),
                prompt_id, "ComfyUI job completed via WS"
            );
            ping_task.abort();
            cancel.cancel();
        }
        "status" => {
            let queue = val["data"]["status"]["exec_info"]["queue_remaining"]
                .as_u64()
                .unwrap_or(0);
            emit(
                "image:queue",
                serde_json::json!({ "queue_remaining": queue }),
            );
        }
        "error" => {
            let message = val["data"]["message"]
                .as_str()
                .unwrap_or("unknown ComfyUI error")
                .to_string();
            let data = val.get("data").cloned().unwrap_or(serde_json::Value::Null);
            let detailed = format!("{message}; frame_data={data}");
            warn!(message = %detailed, "ComfyUI WS error frame");
            *terminal_error = Some(detailed);
            ping_task.abort();
            cancel.cancel();
        }
        _ => {}
    }
}

/// Public version of the history recovery — used by orchestrator when no AppHandle.
pub async fn recover_from_history_pub(
    port: u16,
    prompt_id: &str,
) -> Result<Vec<ComfyOutput>, WsBridgeError> {
    recover_from_history(port, prompt_id).await
}

enum HistoryRecoveryState {
    Outputs(Vec<ComfyOutput>),
    Pending(String),
    TerminalError(String),
}

fn parse_history_outputs(history: &serde_json::Value, prompt_id: &str) -> Vec<ComfyOutput> {
    let mut outputs = Vec::new();

    if let Some(images) = history[prompt_id]["outputs"].as_object() {
        for node_output in images.values() {
            if let Some(imgs) = node_output["images"].as_array() {
                for img in imgs {
                    outputs.push(ComfyOutput {
                        filename: img["filename"].as_str().unwrap_or("").to_string(),
                        subfolder: img["subfolder"].as_str().unwrap_or("").to_string(),
                        output_type: img["type"].as_str().unwrap_or("output").to_string(),
                    });
                }
            }
        }
    }

    outputs
}

fn status_messages_preview(status: &serde_json::Value) -> String {
    let raw = status
        .get("messages")
        .cloned()
        .unwrap_or(serde_json::Value::Null)
        .to_string();
    raw.chars().take(300).collect()
}

fn classify_history_state(history: &serde_json::Value, prompt_id: &str) -> HistoryRecoveryState {
    let Some(prompt_entry) = history.get(prompt_id) else {
        return HistoryRecoveryState::Pending("prompt_id not found in history yet".to_string());
    };

    let outputs = parse_history_outputs(history, prompt_id);
    if !outputs.is_empty() {
        return HistoryRecoveryState::Outputs(outputs);
    }

    let completed = prompt_entry["status"]["completed"]
        .as_bool()
        .unwrap_or(false);
    let status = prompt_entry["status"]["status_str"]
        .as_str()
        .unwrap_or("unknown");

    if completed || status.eq_ignore_ascii_case("error") {
        let messages = status_messages_preview(&prompt_entry["status"]);
        return HistoryRecoveryState::TerminalError(format!(
            "history completed without outputs (status={status}, completed={completed}, messages={messages})"
        ));
    }

    HistoryRecoveryState::Pending(format!(
        "history entry present but outputs are not ready (status={status}, completed={completed})"
    ))
}

async fn recover_from_history_with_deadline(
    port: u16,
    prompt_id: &str,
    deadline: tokio::time::Instant,
) -> Result<Vec<ComfyOutput>, WsBridgeError> {
    let url = format!("http://127.0.0.1:{}/history/{}", port, prompt_id);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let mut last_pending = "history not queried yet".to_string();

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(WsBridgeError::ComfyError {
                message: format!(
                    "no outputs in history before deadline for prompt {prompt_id}; last_state={last_pending}"
                ),
            });
        }

        match client.get(&url).send().await {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.map_err(|e| WsBridgeError::Http {
                    status: e.status().map(|s| s.as_u16()),
                    message: format!("failed reading /history response body: {e}"),
                })?;

                if !status.is_success() {
                    let body_preview: String = body.chars().take(300).collect();
                    return Err(WsBridgeError::Http {
                        status: Some(status.as_u16()),
                        message: format!(
                            "GET /history/{prompt_id} returned non-success: {body_preview}"
                        ),
                    });
                }

                let val = serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
                    let body_preview: String = body.chars().take(300).collect();
                    WsBridgeError::ComfyError {
                        message: format!("history parse failed: {e}; body={body_preview}"),
                    }
                })?;

                match classify_history_state(&val, prompt_id) {
                    HistoryRecoveryState::Outputs(outputs) => return Ok(outputs),
                    HistoryRecoveryState::TerminalError(message) => {
                        return Err(WsBridgeError::ComfyError { message });
                    }
                    HistoryRecoveryState::Pending(note) => {
                        last_pending = note;
                    }
                }
            }
            Err(e) => {
                return Err(WsBridgeError::Http {
                    status: e.status().map(|s| s.as_u16()),
                    message: e.to_string(),
                });
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Poll `GET /history/{prompt_id}` to recover job state after WS disconnect.
async fn recover_from_history(
    port: u16,
    prompt_id: &str,
) -> Result<Vec<ComfyOutput>, WsBridgeError> {
    recover_from_history_with_deadline(
        port,
        prompt_id,
        tokio::time::Instant::now() + Duration::from_secs(30),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_fake_comfy_interrupt_server(
    ) -> (u16, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let interrupts = Arc::new(AtomicUsize::new(0));
        let interrupt_hits = interrupts.clone();

        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let interrupt_hits = interrupt_hits.clone();
                tokio::spawn(async move {
                    let mut buf = [0_u8; 2048];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }

                    let req = String::from_utf8_lossy(&buf[..n]);
                    if req.starts_with("POST /interrupt") {
                        interrupt_hits.fetch_add(1, Ordering::SeqCst);
                    }

                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .await;
                });
            }
        });

        (port, interrupts, handle)
    }

    #[tokio::test]
    async fn receiver_drop_triggers_comfy_interrupt() {
        let (port, interrupt_hits, server_handle) = spawn_fake_comfy_interrupt_server().await;

        let (tx, rx) = oneshot::channel::<Result<Vec<ComfyOutput>, WsBridgeError>>();
        drop(rx);

        let listener = spawn_ws_listener(
            port,
            "client-test".to_string(),
            "prompt-test".to_string(),
            None,
            tx,
            tokio_util::sync::CancellationToken::new(),
            Duration::from_secs(5),
        );

        let _ = tokio::time::timeout(Duration::from_secs(3), listener)
            .await
            .expect("ws listener should exit after receiver drop");

        let hits = interrupt_hits.load(Ordering::SeqCst);
        server_handle.abort();

        assert!(hits >= 1, "expected at least one interrupt request");
    }
}
