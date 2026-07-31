//! Server MemorySystem HTTP surface (memory-upgrade P7).
//!
//! Exposes the unified [`MemorySystem`](kria_core::memory::api::MemorySystem)
//! over HTTP so the server is a first-class memory participant — same authority
//! DB, same retriever/planner/reasoning/graph/library/cognition as the desktop.
//! Every handler delegates to the façade; there is NO server-side memory logic
//! and NO parallel retrieval path. Returns `503` when memory is unavailable.
//!
//! ## F1.6.5 security audit (MGR-003 AC2/AC6, MGR-004 AC4/AC7, design §8.3)
//!
//! Every route this module mounts, audited against the task's named classes:
//!
//! - **search / reason** (`/memory/search`, `/memory/reason`) — read-only,
//!   currently gated only by the `503`-when-unavailable check, not by the
//!   capability lattice. Left unchanged: [`kria_core::memory::contract::search`]/
//!   `reason` and the underlying [`MemorySystem::search`] take no
//!   `CallerContext`/`PolicyPartition` parameter at all — there is no
//!   caller-scoped filtering to gate *to* at this layer, and adding a
//!   route-level `CommandKind`-style "read" capability here would not close
//!   that gap (a permitted caller would still see the whole unfiltered
//!   corpus). This is a **real MGR-004 gap**, but it lives in `kria-core`'s
//!   retrieval/contract layer, not this HTTP adapter — see the follow-up note
//!   below. Design §8.3 marks search/neighborhood/path/inspect/aggregate/
//!   predict/time/trace "Planned supported [remotely] only with full
//!   authn/authz/security", which today means "the global auth/origin/
//!   rate-limit middleware ran" (`lib.rs::build_router`) — satisfied — not
//!   "results are namespace-scoped to the caller", which is not yet true.
//! - **graph / path / prediction** — `/memory/graph` (degree centrality) is
//!   the only one of these three exposed over HTTP today.
//!   [`MemorySystem::graph_neighbors`] (neighborhood/path-adjacent) and
//!   [`MemorySystem::graph_predict_links`] (prediction) exist in
//!   `kria-core` but have **no HTTP route or `contract::*` wrapper** — this
//!   task does not add one (that is F3's canonical API surface, a distinct
//!   future gate; adding routes here would be scope creep). Same unscoped-
//!   read gap as `search`/`reason` applies to `graph` itself.
//! - **trace** — no dedicated `/memory/trace` route exists; the closest
//!   analog, `/memory/explain` (provenance/contradiction/worth trace for one
//!   memory id), has the same unscoped-read posture as `search`.
//! - **patch / SSE replay** — `/memory/events` is the only SSE route. It is
//!   an unfiltered broadcast of every namespace's/scope's changes with no
//!   per-subscriber policy partition, so ANY caller admitted once
//!   `remote_enabled = true` — even one holding every capability grant —
//!   would see cross-namespace activity outside their own scope, a live
//!   MGR-004 leak. Fixed in this task by denying the route outright whenever
//!   this deployment has remote exposure configured (see `events_sse` /
//!   `require_no_remote_exposure`) — the one concrete gap in this task's
//!   named scope that IS fixed here. No revision-patch replay/resume/cursor
//!   endpoint exists yet (design §8.3's "authenticated SSE + replay cursor"
//!   for server remote is aspirational/future — F1.7+).
//! - **command** — there is no generic command-preview/commit HTTP endpoint.
//!   "command" in this task's scope is the existing per-kind mutation routes
//!   (`remember`/`forget`/`delete`/`verify`/`reflect`/`consolidate`), already
//!   gated by `require_capability` since F1.5.3/F1.6.4.
//! - **lifecycle** — `forget`/`hard_delete` are lifecycle writes, already
//!   gated (`CommandKind::Forget`/`HardDelete`). `verify` is gated as
//!   `CommandKind::Correct` (it corrects stored confidence, not a lifecycle
//!   transition). No separate lifecycle-preview endpoint exists yet (F1.7).
//! - **source** — no source-management/ingestion/consent HTTP route exists.
//!   Design §8.3 marks server "source ingest" as "metadata/manual stream
//!   only" (loopback) / "disabled current release" (remote) — consistent
//!   with there being no route to gate yet.
//! - **health** — `/memory/health` and `/memory/metrics` remain ungated
//!   (any caller that reached this router — even unauthenticated-in-default-
//!   loopback — can read them). Judged intentional, not a silent gap: their
//!   payloads are aggregate authority telemetry (schema/API version, total
//!   event/memory counts, tool-outcome seen/persisted/gated counters,
//!   pending-enrichment backlog) with no per-record label, content, or
//!   identifier — the same class of information MGR-028 "privacy-safe
//!   observability" treats as safe-by-design operational telemetry, and
//!   distinct from the "protected count/topology" MGR-003 AC3 forbids in a
//!   **deny** response (these are successful-response aggregate counts, not
//!   denial-shape leakage). Left ungated to keep `/memory/health` usable as
//!   an unauthenticated liveness probe, matching how `/api/health` behaves.
//! - **unsupported local-only routes** — `backup`/`restore` are the only
//!   local-only-by-design operations exposed over HTTP, already gated via
//!   `require_local_desktop_only` for every server caller regardless of
//!   origin. No other route touches raw filesystem paths or whole-authority
//!   operations.
//!
//! **Follow-up (out of this task's scope, documented per the F2-blocker
//! style):** none of `search`/`reason`/`health`/`metrics`/`timeline`/`goals`/
//! `plans`/`reasoning`/`research`/`graph`/`library`/`explain`/`report` apply
//! any caller-scoped `PolicyPartition`/`Effective_Policy` filtering — every
//! `contract::*` read function and the `MemorySystem` methods it delegates to
//! take no caller/policy parameter and query the whole authority DB.
//! Concretely: F1.4.5 already built the designed policy-first read gate —
//! [`kria_core::memory::policy::read_authorization::authorize_read`] /
//! `AuthorizedScope` — but grep confirms it is called from **nowhere** in
//! `contract.rs`, `retriever.rs`, `api.rs`, or `graph_intel.rs` today; the
//! gate exists and is unit-tested in isolation but is not yet wired into any
//! real read path. Today this is safe ONLY because a same-partition-caller
//! assumption holds (every server-adapter caller, `LocalDesktop` or
//! `AuthenticatedRemote`, currently shares one static `[server].caller`
//! partition — see `lib.rs`/`auth.rs`); it stops being safe the moment two
//! distinct callers with different partitions are admitted to the same
//! running server, which MGR-004 AC4/AC7 require the system to handle
//! correctly. Closing this requires a `kria-core` change threading
//! `authorize_read`'s `AuthorizedScope`/`ScopePredicate` through
//! `contract::*` into `Retriever::search`/`RetrievalCtx` and the analytics/
//! graph/library read paths — the gate already exists, it just needs to be
//! called — out of scope for this HTTP-adapter-focused task.

use crate::ServerState;
use axum::{
    extract::{Extension, Query, State},
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
use kria_core::memory::authority::{is_command_capability_permitted, CommandKind};
use kria_core::memory::contract;
use kria_core::memory::model::CallerContext;

use crate::correlation::CorrelationId;
use crate::deny::deny_envelope;

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
///
/// F1.6.5 (MGR-003 AC2/AC6, MGR-004 AC4/AC7, design §8.3 "revision patches …
/// authenticated SSE + replay cursor"): denied outright whenever this
/// deployment has `[server].remote_enabled = true`, via
/// [`require_no_remote_exposure`] — regardless of which `CommandKind` grants
/// the resolved caller otherwise holds.
///
/// This is deliberately NOT gated with [`require_capability`] +
/// `CommandKind::Observe` the way `remember`/`reflect` are: under the current
/// lattice ([`is_command_capability_permitted`]) `AuthenticatedRemote` already
/// permits `Observe`, so that check would be a no-op — it would deny nobody.
/// It is also deliberately NOT keyed on `CallerContext::origin()`: the real
/// server binary (`main.rs`) always constructs `AuthenticatedRemote` for its
/// static adapter-boundary caller, even when bound to loopback — only
/// `remote_enabled` distinguishes "safe local-only deployment" from "exposed
/// to an untrusted network" (see `require_no_remote_exposure` doc comment).
///
/// The actual exposure here is not a missing authorization grant, it is that
/// [`MemorySystem::subscribe_changes()`] is a single unfiltered broadcast of
/// **every** committed change across every namespace/scope/sensitivity with
/// no per-subscriber policy partition applied (`MemoryChange` carries no
/// partition at all). A legitimate, correctly-authenticated remote caller
/// would therefore see every OTHER caller's/namespace's live activity the
/// moment they subscribed — a live MGR-004 cross-namespace leak, not an
/// authentication bypass. Properly scoping the broadcast per-subscriber is a
/// `kria-core` change (tagging `MemoryChange` with its originating
/// `PolicyPartition` and filtering in `notify_change` / the broadcast
/// consumer) that is out of this task's HTTP-adapter scope (see the
/// module-level F1.6.5 audit note). Until that exists, the only *correct*
/// HTTP-adapter-level mitigation is to deny the stream once remote exposure
/// is configured, rather than admit any remote caller to data outside their
/// own scope. The default loopback deployment (`remote_enabled = false`,
/// this crate's safe default — F1.6.1) remains fully permitted — single-user
/// local trust is unaffected.
async fn events_sse(
    State(state): State<Arc<ServerState>>,
    correlation_ext: Option<Extension<CorrelationId>>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, axum::response::Response> {
    if let Err(e) = require_no_remote_exposure(&state, correlation_ext.map(|Extension(c)| c)) {
        return Err(*e);
    }
    let ms = state
        .memory_system
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
        .map_err(|s| s.into_response())?;
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

/// The single non-revealing deny envelope for a mutation capability the
/// caller's origin is not permitted to issue (MGR-003 AC3, design §19.8/§8.3).
/// Every unsupported-capability denial returns this exact shape — no memory
/// id, count, label, topology, or specific-command detail — so an anonymous
/// or under-authorized remote caller cannot distinguish "capability denied"
/// from any other capability denial by response content. Delegates to the
/// crate-wide [`deny_envelope`] (F1.6.4) so this shares its exact field
/// set/order/length class with every other boundary's deny path (auth/
/// origin/rate-limit), and now includes the request's correlation ID the
/// same way those paths already do.
fn capability_denied(correlation_id: Option<CorrelationId>) -> axum::response::Response {
    deny_envelope(StatusCode::FORBIDDEN, "unsupported_capability", correlation_id)
}

/// Reject a capability the server host never supports, regardless of caller
/// origin (design §8.3 capability matrix: `export/import/recovery` is
/// "local desktop only" — "unsupported: local ownership/security" even for a
/// loopback server caller, "unsupported" for a remote one). Full backup/
/// restore is a raw whole-authority file operation with **no** Effective-
/// Policy enforcement (it copies every namespace/scope/sensitivity row
/// byte-for-byte); a server adapter — loopback or remote — has no local-
/// filesystem-ownership guarantee over the resulting file, so this is not a
/// per-caller grant question the origin/kind lattice can decide. Same
/// non-revealing deny shape as [`require_capability`] (MGR-003 AC3).
fn require_local_desktop_only(
    correlation_id: Option<CorrelationId>,
) -> Result<(), Box<axum::response::Response>> {
    Err(Box::new(capability_denied(correlation_id)))
}

/// Deny a route whenever this deployment has remote exposure enabled
/// (`[server].remote_enabled = true`), regardless of which `CommandKind`
/// grants the resolved caller otherwise holds (F1.6.5, MGR-004 AC4/AC7).
///
/// Deliberately keyed on [`ServerState::config`]`.server.remote_enabled` —
/// the SAME boolean `lib.rs::build_router` already uses to decide whether
/// the auth/origin/rate-limit middleware are layered on at all — rather than
/// on `CallerContext::origin()`. Origin cannot distinguish the two
/// deployments that matter here: the real server binary (`main.rs`) always
/// constructs `CallerOrigin::AuthenticatedRemote` for its adapter boundary
/// caller EVEN when bound to loopback (only `remote_enabled` changes), so an
/// origin-keyed check would deny this route unconditionally, including in
/// the safe default loopback deployment — a regression, not a fix. Gating on
/// `remote_enabled` instead denies the route only once this process is
/// actually exposed to an untrusted network, matching every other MGR-003
/// remote-only hardening layer's own gate.
///
/// Unlike [`require_local_desktop_only`] — which denies a host CAPABILITY no
/// server caller may ever invoke (backup/restore touch raw whole-authority
/// files) — this denies a route only once remote exposure is configured, for
/// a route whose underlying data path performs no per-caller policy-
/// partition filtering at all (see `events_sse` doc comment for the concrete
/// case this backs). Same non-revealing deny shape as every other capability
/// boundary in this module (MGR-003 AC3).
fn require_no_remote_exposure(
    state: &Arc<ServerState>,
    correlation_id: Option<CorrelationId>,
) -> Result<(), Box<axum::response::Response>> {
    if state.config.server.remote_enabled {
        Err(Box::new(capability_denied(correlation_id)))
    } else {
        Ok(())
    }
}

/// Gate a durable-mutation route on the caller/command capability lattice
/// (MGR-003 AC2/AC3, design §8.3 capability matrix: "disabled by default;
/// explicit operation grants"). This is the SAME decision
/// [`AuthorityCommandBus`](kria_core::memory::authority::AuthorityCommandBus)
/// applies to every command that already reached the governed bus
/// ([`is_command_capability_permitted`]) — reused here so the pre-authority
/// `/memory/*` mutation routes (still calling `WritePolicy`/`Lifecycle`
/// directly because the F2 per-kind semantic builders do not yet exist,
/// task F1.5.3) reject an unsupported remote mutation *before* touching any
/// store, instead of silently allowing every remote caller to mutate local
/// memory. An `AuthenticatedRemote` caller may only issue
/// [`CommandKind::Observe`]-equivalent operations by default; every other
/// mutation kind requires an explicit operation grant this build does not
/// yet implement.
fn require_capability(
    caller: &CallerContext,
    kind: CommandKind,
    correlation_id: Option<CorrelationId>,
) -> Result<(), Box<axum::response::Response>> {
    if is_command_capability_permitted(caller.origin(), kind) {
        Ok(())
    } else {
        Err(Box::new(capability_denied(correlation_id)))
    }
}

/// Resolve the caller identity that governs THIS request (F1.6.2).
///
/// When the remote bearer-auth middleware ran (remote mode — see
/// `build_router`), it verified a real signed token and inserted a
/// per-request [`CallerContext::authenticated_remote`] built from the
/// token's actual `actor_id`/`device_id` as a request extension; that is the
/// identity used here. When the middleware did not run (default loopback
/// mode, where MGR-003 does not require token auth — see `auth.rs`/`lib.rs`
/// docs), no extension is present and the route falls back to the single
/// static [`ServerState::caller`] the adapter constructed at startup, exactly
/// as before F1.6.2. Either way, every remote-adapter caller still resolves
/// to `CallerOrigin::AuthenticatedRemote`, so the capability lattice below is
/// unaffected by which path supplied the identity.
fn effective_caller<'a>(
    state: &'a Arc<ServerState>,
    per_request: &'a Option<Extension<CallerContext>>,
) -> &'a CallerContext {
    match per_request {
        Some(Extension(caller)) => caller,
        None => &state.caller,
    }
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
    caller_ext: Option<Extension<CallerContext>>,
    correlation_ext: Option<Extension<CorrelationId>>,
    Json(req): Json<RememberReq>,
) -> impl IntoResponse {
    // `remember` is an Observe-kind write — the one mutation an
    // AuthenticatedRemote caller may issue by default (MGR-003 AC2).
    if let Err(e) = require_capability(
        effective_caller(&state, &caller_ext),
        CommandKind::Observe,
        correlation_ext.map(|Extension(c)| c),
    ) {
        return e.into_response();
    }
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
    caller_ext: Option<Extension<CallerContext>>,
    correlation_ext: Option<Extension<CorrelationId>>,
    Json(req): Json<ScopeReq>,
) -> impl IntoResponse {
    // Lifecycle (soft-delete) write — disabled for a remote caller by default
    // (design §8.3 capability matrix: "correction/merge/split/relation/goal/
    // lifecycle … disabled by default; explicit operation grants").
    if let Err(e) = require_capability(
        effective_caller(&state, &caller_ext),
        CommandKind::Forget,
        correlation_ext.map(|Extension(c)| c),
    ) {
        return e.into_response();
    }
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::forget(&ms, &req.kind, &req.value))
}

async fn hard_delete(
    State(state): State<Arc<ServerState>>,
    caller_ext: Option<Extension<CallerContext>>,
    correlation_ext: Option<Extension<CorrelationId>>,
    Json(req): Json<ScopeReq>,
) -> impl IntoResponse {
    // Irreversible lifecycle write — disabled for a remote caller by default,
    // same as `forget`.
    if let Err(e) = require_capability(
        effective_caller(&state, &caller_ext),
        CommandKind::HardDelete,
        correlation_ext.map(|Extension(c)| c),
    ) {
        return e.into_response();
    }
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
    caller_ext: Option<Extension<CallerContext>>,
    correlation_ext: Option<Extension<CorrelationId>>,
    Json(req): Json<IdReq>,
) -> impl IntoResponse {
    // `verify` may demote a memory's stored confidence (Truth Maintenance
    // §22.4) — a correction of existing data, same capability class as
    // `forget`/`hard_delete` in the design §8.3 matrix.
    if let Err(e) = require_capability(
        effective_caller(&state, &caller_ext),
        CommandKind::Correct,
        correlation_ext.map(|Extension(c)| c),
    ) {
        return e.into_response();
    }
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::verify(&ms, &req.id))
}

async fn reflect(
    State(state): State<Arc<ServerState>>,
    caller_ext: Option<Extension<CallerContext>>,
    correlation_ext: Option<Extension<CorrelationId>>,
) -> impl IntoResponse {
    // Reflection/consolidation appends new derived memories through the same
    // Observe-class write path as `remember` — the one mutation an
    // AuthenticatedRemote caller may issue by default.
    if let Err(e) = require_capability(
        effective_caller(&state, &caller_ext),
        CommandKind::Observe,
        correlation_ext.map(|Extension(c)| c),
    ) {
        return e.into_response();
    }
    let ms = match ms(&state) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    respond(contract::reflect(&ms).await)
}

async fn consolidate(
    State(state): State<Arc<ServerState>>,
    caller_ext: Option<Extension<CallerContext>>,
    correlation_ext: Option<Extension<CorrelationId>>,
    Json(req): Json<IdReq>,
) -> impl IntoResponse {
    if let Err(e) = require_capability(
        effective_caller(&state, &caller_ext),
        CommandKind::Observe,
        correlation_ext.map(|Extension(c)| c),
    ) {
        return e.into_response();
    }
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
    correlation_ext: Option<Extension<CorrelationId>>,
    Json(req): Json<BackupReq>,
) -> impl IntoResponse {
    // Full-authority export is local-desktop-only on every server host
    // (design §8.3): it copies every namespace/scope/sensitivity row with no
    // Effective-Policy filter, so no server caller — loopback or remote — may
    // trigger it (MGR-003 AC2).
    if let Err(e) = require_local_desktop_only(correlation_ext.map(|Extension(c)| c)) {
        return e.into_response();
    }
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
    correlation_ext: Option<Extension<CorrelationId>>,
    Json(req): Json<RestoreReq>,
) -> impl IntoResponse {
    // Full-authority import/recovery is local-desktop-only on every server
    // host (design §8.3), same reasoning as `backup`.
    if let Err(e) = require_local_desktop_only(correlation_ext.map(|Extension(c)| c)) {
        return e.into_response();
    }
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
