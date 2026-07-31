//! Mobile prompt-control HTTP routes (Phase 4.5.4).
//!
//! Device pairing + token issuance + revocation. Pairing begin/list/revoke are
//! intended to be reached from the trusted desktop/local UI; `pair/complete` is
//! the one endpoint the phone calls (with the scanned pairing code).
//!
//! Security: these endpoints expose device management, so in production they
//! must sit behind the private mesh + local auth. Tokens and the signing key
//! are never returned by list endpoints.

use crate::gateway::PhoneGatewayState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Html,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn mobile_routes() -> Router<Arc<PhoneGatewayState>> {
    Router::new()
        .route("/pair", get(pair_page))
        .route("/api/mobile/pair/begin", post(pair_begin))
        .route("/api/mobile/pair/complete", post(pair_complete))
        .route("/api/mobile/devices", get(list_devices))
        .route(
            "/api/mobile/devices/{device_id}/revoke",
            post(revoke_device),
        )
}

/// Operator-facing pairing page (open on the laptop). Generates a single-use
/// pairing code + shows the server URL the phone should use. No device token
/// required — this is the bootstrap step, intended for localhost/trusted use.
async fn pair_page() -> Html<&'static str> {
    Html(PAIR_PAGE_HTML)
}

/// Extract a device token from `Authorization: Bearer …` (device-management
/// endpoints are never called with a `?token=` query param — see
/// `remote_desktop_routes::extract_token` for the WS/query-param variant
/// used by client-facing routes only).
fn extract_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.to_string())
}

/// Enforce device-token auth on a device-MANAGEMENT endpoint (list/revoke),
/// when configured (F1.6.6 — MGR-003 AC2/AC6: no route registration bypasses
/// the equivalent-strength device-token boundary the gateway already applies
/// to `remote_desktop_routes`/`ws.rs`).
///
/// `pair/begin` and `pair/complete` are deliberately EXEMPT (see their own
/// doc comments: pairing is the bootstrap step a not-yet-paired phone must
/// reach, and `pair/complete` is itself the credential-issuing operation —
/// gating it on a token would be circular). `list_devices`/`revoke_device`
/// read/mutate the registry of ALREADY-paired devices and have no such
/// bootstrap requirement, so — before this fix — they were reachable by ANY
/// caller that reached the gateway's mesh/LAN listener with no token at all,
/// even though the sibling `remote_desktop_routes::authorize` already gates
/// its own analogous control-plane endpoints (`request`/`confirm`/`stop`/
/// `status`) on exactly this same check. This closes that inconsistency
/// rather than inventing a new security model: same gate, same config flag,
/// same non-revealing `401` shape as `remote_desktop_routes::authorize`.
fn authorize_device_management(
    state: &Arc<PhoneGatewayState>,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if !state.config.mobile.require_device_auth {
        return Ok(());
    }
    let Some(registry) = state.device_registry.as_ref() else {
        return Ok(());
    };
    match extract_token(headers).map(|t| registry.verify_token(&t)) {
        Some(Ok(_)) => Ok(()),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(
                serde_json::json!({ "status": "error", "message": "valid device token required" }),
            ),
        )),
    }
}

const PAIR_PAGE_HTML: &str = r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="UTF-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1.0"/>
<title>KRIA — Pair a device</title>
<style>
  body{font-family:system-ui,sans-serif;background:#0b0f14;color:#e2e8f0;display:flex;
    min-height:100vh;align-items:center;justify-content:center;margin:0}
  .card{background:#111827;border:1px solid #1e293b;border-radius:16px;padding:28px;max-width:420px;width:90%}
  h1{font-size:18px;margin:0 0 8px}
  .code{font-size:34px;font-weight:700;letter-spacing:2px;color:#38bdf8;word-break:break-all;
    background:#0b0f14;padding:14px;border-radius:10px;text-align:center;margin:14px 0}
  .row{font-size:13px;color:#94a3b8;margin:6px 0}
  .val{color:#e2e8f0;word-break:break-all}
  button{margin-top:14px;width:100%;padding:12px;border:none;border-radius:10px;
    background:#38bdf8;color:#0b0f14;font-weight:600;font-size:15px;cursor:pointer}
  .err{color:#fecaca;background:#7f1d1d;padding:10px;border-radius:8px;font-size:13px}
</style></head><body>
<div class="card">
  <h1>Pair a phone with KRIA</h1>
  <div class="row">On your phone, open <span class="val" id="murl"></span> and enter:</div>
  <div class="code" id="code">…</div>
  <div class="row">Server URL to enter on phone: <span class="val" id="surl"></span></div>
  <div class="row" id="exp"></div>
  <div class="err" id="err" style="display:none"></div>
  <button onclick="gen()">Generate new code</button>
</div>
<script>
  function origin(){return window.location.origin}
  async function gen(){
    document.getElementById('err').style.display='none';
    try{
      const r=await fetch('/api/mobile/pair/begin',{method:'POST',
        headers:{'Content-Type':'application/json'},body:JSON.stringify({})});
      const d=await r.json();
      if(!r.ok){throw new Error(d.message||('status '+r.status))}
      document.getElementById('code').textContent=d.code;
      const exp=new Date(d.expires_at*1000).toLocaleTimeString();
      document.getElementById('exp').textContent='Code expires at '+exp;
    }catch(e){
      const el=document.getElementById('err');el.style.display='block';
      el.textContent='Could not generate code: '+e.message+
        ' (is [mobile].enabled = true?)';
    }
  }
  document.getElementById('murl').textContent=origin()+'/m';
  document.getElementById('surl').textContent=origin();
  gen();
</script>
</body></html>"#;

fn registry(
    state: &Arc<PhoneGatewayState>,
) -> Result<&Arc<kria_core::mobile::DeviceRegistry>, (StatusCode, Json<serde_json::Value>)> {
    state.device_registry.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "message": "mobile prompt-control is disabled (set [mobile].enabled = true)",
            })),
        )
    })
}

#[derive(serde::Deserialize)]
struct PairBeginRequest {
    /// Host:port the phone should connect to (the tailnet address of this laptop).
    #[serde(default)]
    host: Option<String>,
}

async fn pair_begin(
    State(state): State<Arc<PhoneGatewayState>>,
    body: Option<Json<PairBeginRequest>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let reg = registry(&state)?;
    let host = body
        .and_then(|b| b.0.host)
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| {
            let cfg = &state.config;
            let h = if cfg.mobile.bind_interface.is_empty() {
                cfg.server.host.clone()
            } else {
                cfg.mobile.bind_interface.clone()
            };
            format!("{h}:{}", cfg.server.port)
        });
    let challenge = reg.begin_pairing(&host);
    Ok(Json(serde_json::json!({
        "status": "ok",
        "code": challenge.code,
        "qr_payload": challenge.qr_payload,
        "expires_at": challenge.expires_at,
    })))
}

#[derive(serde::Deserialize)]
struct PairCompleteRequest {
    code: String,
    #[serde(default = "default_device_name")]
    device_name: String,
}

fn default_device_name() -> String {
    "Unnamed device".to_string()
}

async fn pair_complete(
    State(state): State<Arc<PhoneGatewayState>>,
    Json(req): Json<PairCompleteRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let reg = registry(&state)?;
    match reg.complete_pairing(req.code.trim(), req.device_name.trim()) {
        Ok((info, token)) => Ok(Json(serde_json::json!({
            "status": "ok",
            "token": token,
            "device": {
                "id": info.id,
                "name": info.name,
                "created_at": info.created_at,
            },
        }))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
        )),
    }
}

async fn list_devices(
    State(state): State<Arc<PhoneGatewayState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorize_device_management(&state, &headers)?;
    let reg = registry(&state)?;
    let devices = reg.list_devices().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
        )
    })?;
    Ok(Json(
        serde_json::json!({ "status": "ok", "devices": devices }),
    ))
}

async fn revoke_device(
    State(state): State<Arc<PhoneGatewayState>>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    authorize_device_management(&state, &headers)?;
    let reg = registry(&state)?;
    let existed = reg.revoke(&device_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
        )
    })?;
    Ok(Json(serde_json::json!({
        "status": "ok",
        "device_id": device_id,
        "revoked": existed,
    })))
}
