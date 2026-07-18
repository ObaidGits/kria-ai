//! Server MemorySystem HTTP surface (memory-upgrade P7).
//!
//! Exposes the unified [`MemorySystem`](kria_core::memory::api::MemorySystem)
//! over HTTP so the server is a first-class memory participant — same authority
//! DB, same retriever/planner/reasoning/graph/library/cognition as the desktop.
//! Every handler delegates to the façade; there is NO server-side memory logic
//! and NO parallel retrieval path. Returns `503` when memory is unavailable.

use crate::ServerState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use kria_core::memory::api::MemorySystem;
use kria_core::memory::contract;

pub fn memory_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/memory/events", get(events_sse))
        .route("/memory/search", get(search).post(search))
        .route("/memory/reason", get(reason))
        .route("/memory/remember", post(remember))
        .route("/memory/forget", post(forget))
        .route("/memory/delete", post(hard_delete))
        .route("/memory/verify", post(verify))
        .route("/memory/reflect", post(reflect))
        .route("/memory/consolidate", post(consolidate))
        .route("/memory/health", get(health))
        .route("/memory/metrics", get(metrics))
        .route("/memory/timeline", get(timeline))
        .route("/memory/goals", get(goals))
        .route("/memory/plans", get(plans))
        .route("/memory/reasoning", get(reasoning))
        .route("/memory/research", get(research))
        .route("/memory/graph", get(graph))
        .route("/memory/library", get(library))
        .route("/memory/explain", get(explain))
        .route("/memory/report", get(report))
        .route("/memory/backup", post(backup))
        .route("/memory/restore", post(restore))
}

/// GET `/memory/events` — Server-Sent Events stream of live memory changes
/// (UI-1). Forwards `MemorySystem::subscribe_changes()` so remote clients get
/// the same real-time memory activity (created/reflection/library/goal/…) the
/// desktop gets via Tauri events. Returns `503` when memory is unavailable.
async fn events_sse(
    State(state): State<Arc<ServerState>>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let ms = state
        .memory_system
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut rx = ms.subscribe_changes();
    let stream = async_stream::stream! {
        // Announce the stream is live so a client knows it is subscribed.
        yield Ok(Event::default()
            .event("ready")
            .data(serde_json::json!({"ok": true}).to_string()));
        loop {
            match rx.recv().await {
                Ok(change) => {
                    let payload = serde_json::json!({
                        "kind": change.kind,
                        "detail": change.detail,
                    });
                    yield Ok(Event::default().event("memory").data(payload.to_string()));
                }
                // Slow consumer fell behind — keep the stream alive, skip missed.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    yield Ok(Event::default()
                        .event("lagged")
                        .data(serde_json::json!({"missed": n}).to_string()));
                }
                // Sender dropped (shutdown) — end the stream.
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    ))
}

/// Extract the memory system or return a 503 body.
fn ms(
    state: &Arc<ServerState>,
) -> Result<Arc<MemorySystem>, (StatusCode, Json<serde_json::Value>)> {
    state.memory_system.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "memory system unavailable" })),
        )
    })
}

fn err(e: impl std::fmt::Display) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

#[derive(serde::Deserialize)]
struct QueryReq {
    q: String,
    limit: Option<usize>,
}

/// Map a contract `MemoryResult<Value>` into an HTTP response.
fn respond(
    r: kria_core::memory::error::MemoryResult<serde_json::Value>,
) -> axum::response::Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(e).into_response(),
    }
}

async fn search(
    State(state): State<Arc<ServerState>>,
    Query(req): Query<QueryReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::search(&ms, &req.q, req.limit.unwrap_or(20)).await)
}

async fn reason(
    State(state): State<Arc<ServerState>>,
    Query(req): Query<QueryReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::reason(&ms, &req.q, req.limit.unwrap_or(20)).await)
}

#[derive(serde::Deserialize)]
struct RememberReq {
    text: String,
}

async fn remember(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RememberReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::remember(&ms, req.text))
}

#[derive(serde::Deserialize)]
struct ScopeReq {
    kind: String,
    value: String,
}

async fn forget(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ScopeReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::forget(&ms, &req.kind, &req.value))
}

async fn hard_delete(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ScopeReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::hard_delete(&ms, &req.kind, &req.value).await)
}

#[derive(serde::Deserialize)]
struct IdReq {
    id: String,
}

async fn verify(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<IdReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::verify(&ms, &req.id))
}

async fn reflect(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::reflect(&ms).await)
}

async fn consolidate(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<IdReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::consolidate(&ms, &req.id).await)
}

async fn health(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::health(&ms).await)
}

async fn metrics(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::metrics(&ms))
}

#[derive(serde::Deserialize)]
struct LimitReq {
    limit: Option<usize>,
}

async fn timeline(
    State(state): State<Arc<ServerState>>,
    Query(req): Query<LimitReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::timeline(&ms, req.limit.unwrap_or(200)))
}

async fn goals(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::goals(&ms, 100))
}

async fn plans(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::plans(&ms))
}

async fn reasoning(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::reasoning(&ms))
}

async fn research(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::research(&ms))
}

async fn graph(
    State(state): State<Arc<ServerState>>,
    Query(req): Query<LimitReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::graph(&ms, req.limit.unwrap_or(50)))
}

async fn library(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::library(&ms))
}

async fn explain(
    State(state): State<Arc<ServerState>>,
    Query(req): Query<IdReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    match contract::explain(&ms, &req.id) {
        Ok(Some(v)) => Json(v).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "not found" })),
        )
            .into_response(),
        Err(e) => err(e).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct BackupReq {
    dest: String,
}

async fn backup(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<BackupReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    let dest = req.dest.clone();
    let res = tokio::task::spawn_blocking(move || ms.backup(&dest)).await;
    match res {
        Ok(Ok(bytes)) => {
            Json(serde_json::json!({ "dest": req.dest, "bytes": bytes })).into_response()
        }
        Ok(Err(e)) => err(e).into_response(),
        Err(e) => err(format!("backup task join error: {e}")).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct RestoreReq {
    src: String,
}

async fn restore(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RestoreReq>,
) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    let src = req.src.clone();
    let res = tokio::task::spawn_blocking(move || ms.restore(&src)).await;
    match res {
        Ok(Ok(())) => Json(serde_json::json!({ "restored": true, "src": req.src })).into_response(),
        Ok(Err(e)) => err(e).into_response(),
        Err(e) => err(format!("restore task join error: {e}")).into_response(),
    }
}

async fn report(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::report(&ms))
}
