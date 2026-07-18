//! Remote desktop view & takeover routes (Phase 4.6).
//!
//! Control plane (request / confirm / stop / status) plus the `/rd-signal`
//! WebRTC signaling endpoint that negotiates the in-app desktop stream.
//!
//! Capture is via xdg-desktop-portal ScreenCast + PipeWire (no RDP / no
//! gnome-remote-desktop), streamed to the browser over WebRTC (DTLS-SRTP).
//! Input is injected via the portal RemoteDesktop grant. Everything is
//! device-token gated (when mobile auth is on), only proceeds while a session
//! is Active and the global halt is clear, and connect/disconnect are audited
//! with the device identity.
//!
//! Phase 5 status: control plane + signaling gate are live; the WebRTC
//! offer/answer + media pipeline land in Phase 6 (`signaling` module).

use crate::gateway::PhoneGatewayState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;

pub fn remote_desktop_routes() -> Router<Arc<PhoneGatewayState>> {
    Router::new()
        .route("/api/remote-desktop/request", post(request_session))
        .route("/api/remote-desktop/confirm", post(confirm_session))
        .route("/api/remote-desktop/stop", post(stop_session))
        .route("/api/remote-desktop/status", get(status))
        .route("/rd-signal", get(rd_signal))
}

/// Extract a device token from `Authorization: Bearer …` or `?token=`.
fn extract_token(headers: &HeaderMap, query_token: &Option<String>) -> Option<String> {
    if let Some(v) = headers.get("Authorization").and_then(|h| h.to_str().ok()) {
        if let Some(tok) = v.strip_prefix("Bearer ") {
            return Some(tok.to_string());
        }
    }
    query_token.clone()
}

/// Enforce device-token auth when configured. Returns the device id (or None
/// when auth is not required) or an error response.
fn authorize(
    state: &Arc<PhoneGatewayState>,
    headers: &HeaderMap,
    query_token: &Option<String>,
) -> Result<Option<String>, (StatusCode, Json<serde_json::Value>)> {
    if !state.config.mobile.require_device_auth {
        return Ok(None);
    }
    let Some(registry) = state.device_registry.as_ref() else {
        return Ok(None);
    };
    let token = extract_token(headers, query_token);
    match token.as_deref().map(|t| registry.verify_token(t)) {
        Some(Ok(id)) => Ok(Some(id)),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(
                serde_json::json!({ "status": "error", "message": "valid device token required" }),
            ),
        )),
    }
}

fn manager(
    state: &Arc<PhoneGatewayState>,
) -> Result<
    &Arc<kria_core::remote_desktop::RemoteDesktopManager>,
    (StatusCode, Json<serde_json::Value>),
> {
    state.remote_desktop.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "message": "remote desktop is disabled (set [remote_desktop].enabled = true)",
            })),
        )
    })
}

async fn request_session(
    State(state): State<Arc<PhoneGatewayState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers, &None)?;
    let mgr = manager(&state)?;
    let id = mgr.request().map_err(bad_request)?;
    Ok(Json(serde_json::json!({
        "status": "ok",
        "session_id": id,
        "requires_confirmation": true,
        "description": "Starting a remote-desktop session grants full live view and \
                        control of this machine's screen. Confirm to start; a kill \
                        switch and idle auto-expiry remain available.",
    })))
}

#[derive(serde::Deserialize)]
struct ConfirmRequest {
    session_id: String,
}

async fn confirm_session(
    State(state): State<Arc<PhoneGatewayState>>,
    headers: HeaderMap,
    Json(req): Json<ConfirmRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers, &None)?;
    let mgr = manager(&state)?;
    let activation = mgr.confirm(req.session_id.trim()).map_err(bad_request)?;
    Ok(Json(serde_json::json!({
        "status": "ok",
        "session_id": activation.session_id,
    })))
}

async fn stop_session(
    State(state): State<Arc<PhoneGatewayState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers, &None)?;
    let mgr = manager(&state)?;
    mgr.stop();
    Ok(Json(serde_json::json!({ "status": "ok", "stopped": true })))
}

async fn status(
    State(state): State<Arc<PhoneGatewayState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorize(&state, &headers, &None)?;
    let mgr = manager(&state)?;
    Ok(Json(serde_json::json!({
        "status": "ok",
        "session": mgr.status(),
    })))
}

fn bad_request(
    e: kria_core::remote_desktop::RemoteDesktopError,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
    )
}

// ── WebRTC signaling — in-app desktop stream (portal ScreenCast + PipeWire) ──

#[derive(Debug, serde::Deserialize)]
struct SignalQuery {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    /// Optional per-connection stream overrides (UX quality selector). When
    /// absent the server's configured defaults are used (byte-compatible).
    #[serde(default)]
    max_dim: Option<u32>,
    #[serde(default)]
    max_fps: Option<u32>,
    #[serde(default)]
    encoder: Option<String>,
}

/// Apply optional, sanitized quality overrides on top of the manager defaults.
/// Clamps to safe bounds and ignores unknown encoders so a malformed query can
/// never break the pipeline.
fn sanitize_quality(
    base: (u32, u32, String),
    max_dim: Option<u32>,
    max_fps: Option<u32>,
    encoder: Option<String>,
) -> (u32, u32, String) {
    let (mut dim, mut fps, mut enc) = base;
    if let Some(d) = max_dim {
        // 0 = native (no cap); otherwise clamp the longest-edge cap to ≤ 3840.
        dim = if d == 0 { 0 } else { d.clamp(240, 3840) };
    }
    if let Some(f) = max_fps {
        fps = f.clamp(1, 60);
    }
    if let Some(e) = encoder {
        let e = e.to_ascii_lowercase();
        if matches!(e.as_str(), "vp8" | "vp9" | "h264") {
            enc = e;
        }
    }
    (dim, fps, enc)
}

async fn rd_signal(
    ws: WebSocketUpgrade,
    State(state): State<Arc<PhoneGatewayState>>,
    headers: HeaderMap,
    Query(query): Query<SignalQuery>,
) -> impl IntoResponse {
    let device_id = match authorize(&state, &headers, &query.token) {
        Ok(id) => id,
        Err(_) => return (StatusCode::UNAUTHORIZED, "device token required").into_response(),
    };
    let Some(mgr) = state.remote_desktop.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "remote desktop disabled").into_response();
    };
    let Some(backend) = state.remote_desktop_backend.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "stream backend unavailable",
        )
            .into_response();
    };
    let session_id = query.session_id.clone().unwrap_or_default();
    if !mgr.validate_session(&session_id) || !mgr.relay_allowed() {
        return (StatusCode::FORBIDDEN, "no active remote-desktop session").into_response();
    }
    // Resolve per-connection quality: manager defaults + sanitized client overrides.
    let quality = sanitize_quality(
        mgr.stream_config(),
        query.max_dim,
        query.max_fps,
        query.encoder.clone(),
    );
    ws.on_upgrade(move |socket| handle_signal_socket(socket, mgr, backend, device_id, quality))
        .into_response()
}

/// Browser→server signaling message (also carries input events).
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientSignal {
    Answer {
        sdp: String,
    },
    Ice {
        sdp_mline_index: u32,
        candidate: String,
    },
    Input(kria_core_input::InputEventWire),
}

// Local alias so the input event type is shared with the backend without a
// circular-looking import.
mod kria_core_input {
    pub use crate::desktop_stream::input::InputEvent as InputEventWire;
}

async fn handle_signal_socket(
    socket: WebSocket,
    mgr: Arc<kria_core::remote_desktop::RemoteDesktopManager>,
    backend: Arc<crate::desktop_stream::PortalWebRtcBackend>,
    device_id: Option<String>,
    quality: (u32, u32, String),
) {
    use crate::desktop_stream::pipeline::{self, SignalOut};

    mgr.audit_relay(true, device_id.as_deref());
    tracing::info!(device = ?device_id, "rd-signal connected");

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<SignalOut>();

    // Server is the offerer: build the pipeline now (fresh PipeWire fd). It will
    // fire on-negotiation-needed → create-offer → SignalOut::Offer to the client.
    let pipe: Option<pipeline::PipelineHandle> = match backend.open_pipewire_fd() {
        Ok(fd) => {
            let cap = backend
                .capture()
                .unwrap_or(crate::desktop_stream::CaptureInfo {
                    node_id: 0,
                    width: 0,
                    height: 0,
                });
            let (max_dim, max_fps, enc) = quality.clone();
            tracing::info!(
                node_id = cap.node_id,
                "[STEP 4/5] rd-signal: pipewire fd acquired, building pipeline (offerer)"
            );
            match pipeline::spawn(
                fd,
                cap.node_id,
                cap.width,
                cap.height,
                max_dim,
                max_fps,
                &enc,
                out_tx.clone(),
            ) {
                Ok(h) => Some(h),
                Err(e) => {
                    tracing::warn!(error = %e, "rd-signal: pipeline build failed");
                    let _ = ws_tx
                        .send(Message::Text(
                            serde_json::json!({ "type": "error", "message": e })
                                .to_string()
                                .into(),
                        ))
                        .await;
                    None
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "rd-signal: open pipewire fd failed");
            let _ = ws_tx
                .send(Message::Text(
                    serde_json::json!({ "type": "error", "message": e })
                        .to_string()
                        .into(),
                ))
                .await;
            None
        }
    };

    loop {
        tokio::select! {
            // Pipeline → browser (offer / ICE).
            Some(out) = out_rx.recv() => {
                let json = match out {
                    SignalOut::Offer(sdp) => { tracing::info!("[STEP 10] rd-signal: → client offer"); serde_json::json!({ "type": "offer", "sdp": sdp }) },
                    SignalOut::Ice { sdp_mline_index, candidate } =>
                        serde_json::json!({ "type": "ice", "sdp_mline_index": sdp_mline_index, "candidate": candidate }),
                    SignalOut::Failed(msg) => { tracing::warn!(%msg, "rd-signal: pipeline failed"); serde_json::json!({ "type": "error", "message": msg }) },
                };
                if ws_tx.send(Message::Text(json.to_string().into())).await.is_err() {
                    break;
                }
            }
            // Browser → server.
            msg = ws_rx.next() => {
                let Some(Ok(msg)) = msg else { break };
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => break,
                    _ => continue,
                };
                mgr.touch();
                let Ok(sig) = serde_json::from_str::<ClientSignal>(&text) else { continue };
                match sig {
                    ClientSignal::Input(ev) => backend.send_input(ev),
                    ClientSignal::Answer { sdp } => {
                        tracing::info!("[STEP 11] rd-signal: client answer received");
                        if let Some(h) = pipe.as_ref() {
                            h.set_answer(&sdp);
                        }
                    }
                    ClientSignal::Ice { sdp_mline_index, candidate } => {
                        if let Some(h) = pipe.as_ref() {
                            h.add_ice(sdp_mline_index, &candidate);
                        }
                    }
                }
            }
            else => break,
        }
    }

    // Teardown: dropping the handle stops the pipeline + closes the fd.
    drop(pipe);
    mgr.audit_relay(false, device_id.as_deref());
    tracing::info!(device = ?device_id, "rd-signal disconnected");
}

#[cfg(test)]
mod tests {
    use super::sanitize_quality;

    fn base() -> (u32, u32, String) {
        (1600, 30, "vp8".to_string())
    }

    #[test]
    fn no_overrides_preserves_defaults() {
        assert_eq!(sanitize_quality(base(), None, None, None), base());
    }

    #[test]
    fn clamps_dimension_and_fps() {
        assert_eq!(
            sanitize_quality(base(), Some(99_999), Some(999), Some("vp9".into())),
            (3840, 60, "vp9".to_string())
        );
        assert_eq!(
            sanitize_quality(base(), Some(10), Some(0), None),
            (240, 1, "vp8".to_string())
        );
    }

    #[test]
    fn zero_dimension_means_native() {
        assert_eq!(
            sanitize_quality(base(), Some(0), None, None),
            (0, 30, "vp8".to_string())
        );
    }

    #[test]
    fn rejects_unknown_encoder() {
        // Unknown encoder falls back to the base value.
        assert_eq!(
            sanitize_quality(base(), None, None, Some("av1".into())),
            (1600, 30, "vp8".to_string())
        );
        // Case-insensitive accept of a known encoder.
        assert_eq!(
            sanitize_quality(base(), None, None, Some("H264".into())),
            (1600, 30, "h264".to_string())
        );
    }
}
