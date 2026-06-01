//! API-side HITL (Human-In-The-Loop) delivery for non-streaming clients.
//!
//! # Problem
//!
//! The HitlGateway in kria-core delivers HITL requests via tokio channels to
//! whoever is listening. The Tauri UI listens via event channels. But external
//! clients (eval scripts, n8n integrations, REST API users) have no way to:
//!   - Receive HITL prompts when the agent suspends
//!   - Send approval/denial responses
//!   - See workflow telemetry events
//!
//! # Solution
//!
//! Two complementary mechanisms:
//!
//! 1. **SSE streaming** at `GET /api/chat/stream?session_id=...` — for clients
//!    that can hold a long-lived HTTP connection and stream telemetry events.
//!
//! 2. **Polling endpoints** for clients that prefer request/response:
//!    - `GET /api/hitl/pending?session_id=...` — list pending HITL requests
//!    - `POST /api/hitl/respond` — submit an approve/deny response
//!
//! # Security
//!
//! All endpoints require the Bearer token (via `api_auth` middleware).
//! HITL responses are validated server-side against pre-emitted option IDs
//! (P11-8) — the backend rejects responses with unknown option IDs.

use axum::extract::{Query, State as AxumState};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

/// A single HITL request awaiting user response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingHitlRequest {
    pub request_id: String,
    pub session_id: String,
    pub action: String,
    pub risk_level: String,
    pub parameters: serde_json::Value,
    pub created_at_ms: u64,
    /// Allowed option IDs (e.g. ["approve", "deny"] or recovery option IDs).
    /// Responses with unknown option IDs are rejected.
    pub allowed_option_ids: Vec<String>,
}

/// Response to a HITL request.
#[derive(Debug, Clone, Deserialize)]
pub struct HitlResponseRequest {
    pub request_id: String,
    pub option_id: String,
    /// Optional free-form reason / parameters
    #[serde(default)]
    #[allow(dead_code)] // Reserved for future use (audit logging, fine-grained responses)
    pub note: Option<String>,
}

/// Validation outcome.
#[derive(Debug, Serialize)]
pub enum HitlValidation {
    Accepted { accepted_at_ms: u64 },
    Rejected { reason: String },
}

/// Global store of pending HITL requests, keyed by `request_id`.
/// Cleaned up on response or expiry.
pub struct HitlStore {
    pending: Arc<RwLock<HashMap<String, PendingHitlRequest>>>,
    /// Channel for sending response back to the agent
    response_routes: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<HitlResponseRequest>>>>,
    /// Process start time for monotonic timestamps
    start_time: Instant,
}

impl HitlStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
            response_routes: Arc::new(Mutex::new(HashMap::new())),
            start_time: Instant::now(),
        })
    }

    /// Register a pending HITL request and obtain a oneshot receiver
    /// for the eventual response. The agent awaits on this receiver.
    ///
    /// Currently used by the API HITL flow when AgentLoop emits HITL events
    /// that need API delivery (e.g., when called via /api/chat from external
    /// clients). For Tauri UI flows, the existing HitlGateway is used directly.
    #[allow(dead_code)] // Reserved for future API HITL bridge integration
    pub async fn register(
        &self,
        session_id: String,
        action: String,
        risk_level: String,
        parameters: serde_json::Value,
        allowed_option_ids: Vec<String>,
    ) -> (String, tokio::sync::oneshot::Receiver<HitlResponseRequest>) {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();

        let pending = PendingHitlRequest {
            request_id: request_id.clone(),
            session_id,
            action,
            risk_level,
            parameters,
            created_at_ms: self.start_time.elapsed().as_millis() as u64,
            allowed_option_ids,
        };

        self.pending
            .write()
            .await
            .insert(request_id.clone(), pending);
        self.response_routes
            .lock()
            .await
            .insert(request_id.clone(), tx);

        info!(
            target: "api_hitl",
            request_id = %request_id,
            "HITL request registered for API delivery"
        );

        (request_id, rx)
    }

    /// List all pending HITL requests, optionally filtered by session.
    pub async fn pending(&self, session_id: Option<&str>) -> Vec<PendingHitlRequest> {
        let guard = self.pending.read().await;
        guard
            .values()
            .filter(|req| match session_id {
                Some(sid) => req.session_id == sid,
                None => true,
            })
            .cloned()
            .collect()
    }

    /// Submit a response. Validates the option_id against allowed_option_ids
    /// and routes it to the waiting oneshot receiver.
    pub async fn respond(&self, response: HitlResponseRequest) -> HitlValidation {
        // Look up the pending request
        let pending = match self.pending.read().await.get(&response.request_id).cloned() {
            Some(p) => p,
            None => {
                return HitlValidation::Rejected {
                    reason: format!("Unknown request_id: {}", response.request_id),
                };
            }
        };

        // Validate option_id
        if !pending.allowed_option_ids.contains(&response.option_id) {
            warn!(
                target: "api_hitl",
                request_id = %response.request_id,
                attempted = %response.option_id,
                allowed = ?pending.allowed_option_ids,
                "HITL response REJECTED: option_id not in allowed list"
            );
            return HitlValidation::Rejected {
                reason: format!(
                    "Option '{}' is not in the allowed set: {:?}",
                    response.option_id, pending.allowed_option_ids
                ),
            };
        }

        // Remove from pending and route to oneshot
        self.pending.write().await.remove(&response.request_id);
        let tx = self
            .response_routes
            .lock()
            .await
            .remove(&response.request_id);

        match tx {
            Some(tx) => {
                if tx.send(response).is_err() {
                    warn!(
                        target: "api_hitl",
                        "Response channel closed (agent dropped)"
                    );
                }
                HitlValidation::Accepted {
                    accepted_at_ms: self.start_time.elapsed().as_millis() as u64,
                }
            }
            None => HitlValidation::Rejected {
                reason: "Response channel not found (already responded?)".to_string(),
            },
        }
    }

    /// Expire requests older than `max_age_secs`. Should be called periodically
    /// by a background task to prevent memory leaks.
    pub async fn expire_old(&self, max_age_secs: u64) -> usize {
        let now_ms = self.start_time.elapsed().as_millis() as u64;
        let cutoff_ms = now_ms.saturating_sub(max_age_secs * 1000);

        let to_remove: Vec<String> = {
            let guard = self.pending.read().await;
            guard
                .iter()
                .filter(|(_, req)| req.created_at_ms < cutoff_ms)
                .map(|(id, _)| id.clone())
                .collect()
        };

        let count = to_remove.len();
        if count > 0 {
            let mut pending = self.pending.write().await;
            let mut routes = self.response_routes.lock().await;
            for id in to_remove {
                pending.remove(&id);
                routes.remove(&id);
            }
            info!(
                target: "api_hitl",
                expired = count,
                "Expired old HITL requests"
            );
        }

        count
    }
}

/// State carrying the HITL store for handlers.
#[derive(Clone)]
pub struct HitlApiState {
    pub store: Arc<HitlStore>,
}

#[derive(Debug, Deserialize)]
pub struct HitlPendingQuery {
    pub session_id: Option<String>,
}

/// `GET /api/hitl/pending?session_id=...` — list pending HITL requests.
pub async fn list_pending_handler(
    AxumState(state): AxumState<HitlApiState>,
    Query(query): Query<HitlPendingQuery>,
) -> Json<serde_json::Value> {
    let pending = state.store.pending(query.session_id.as_deref()).await;
    Json(serde_json::json!({
        "pending": pending,
        "count": pending.len(),
    }))
}

/// `POST /api/hitl/respond` — submit a HITL response.
pub async fn respond_handler(
    AxumState(state): AxumState<HitlApiState>,
    Json(response): Json<HitlResponseRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let validation = state.store.respond(response).await;
    match validation {
        HitlValidation::Accepted { accepted_at_ms } => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "accepted",
                "accepted_at_ms": accepted_at_ms,
            })),
        ),
        HitlValidation::Rejected { reason } => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "rejected",
                "reason": reason,
            })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn store_register_and_list() {
        let store = HitlStore::new();
        let (rid, _rx) = store
            .register(
                "session-1".into(),
                "delete_file".into(),
                "RED".into(),
                serde_json::json!({"path": "/tmp/foo"}),
                vec!["approve".into(), "deny".into()],
            )
            .await;

        let pending = store.pending(Some("session-1")).await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request_id, rid);
        assert_eq!(pending[0].action, "delete_file");
    }

    #[tokio::test]
    async fn store_respond_with_valid_option() {
        let store = HitlStore::new();
        let (rid, mut rx) = store
            .register(
                "session-1".into(),
                "delete_file".into(),
                "RED".into(),
                serde_json::json!({}),
                vec!["approve".into(), "deny".into()],
            )
            .await;

        let resp = HitlResponseRequest {
            request_id: rid.clone(),
            option_id: "approve".into(),
            note: None,
        };
        let validation = store.respond(resp).await;
        assert!(matches!(validation, HitlValidation::Accepted { .. }));

        // The receiver should now receive the response
        let received = rx.try_recv().unwrap();
        assert_eq!(received.option_id, "approve");
    }

    #[tokio::test]
    async fn store_respond_rejects_unknown_option() {
        let store = HitlStore::new();
        let (rid, _rx) = store
            .register(
                "session-1".into(),
                "delete_file".into(),
                "RED".into(),
                serde_json::json!({}),
                vec!["approve".into(), "deny".into()],
            )
            .await;

        let resp = HitlResponseRequest {
            request_id: rid,
            option_id: "delete_all".into(), // NOT in allowed list
            note: None,
        };
        let validation = store.respond(resp).await;
        match validation {
            HitlValidation::Rejected { reason } => {
                assert!(reason.contains("not in the allowed set"));
            }
            _ => panic!("Expected rejection"),
        }
    }

    #[tokio::test]
    async fn store_respond_rejects_unknown_request_id() {
        let store = HitlStore::new();
        let resp = HitlResponseRequest {
            request_id: "nonexistent".into(),
            option_id: "approve".into(),
            note: None,
        };
        let validation = store.respond(resp).await;
        assert!(matches!(validation, HitlValidation::Rejected { .. }));
    }

    #[tokio::test]
    async fn store_expires_old_requests() {
        let store = HitlStore::new();
        let (rid1, _rx1) = store
            .register(
                "session-1".into(),
                "action1".into(),
                "GREEN".into(),
                serde_json::json!({}),
                vec!["approve".into()],
            )
            .await;

        // Sleep just a moment, then expire requests older than 0 secs (all of them)
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        let count = store.expire_old(0).await;
        assert_eq!(count, 1);

        let pending = store.pending(None).await;
        assert!(pending.is_empty());
        assert!(!pending.iter().any(|p| p.request_id == rid1));
    }
}

// ─── SSE streaming endpoint ──────────────────────────────────────────────────
// Provides server-sent events for clients that want to track HITL prompts +
// session state over a long-lived HTTP connection.

use async_stream::stream;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::convert::Infallible;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct HitlStreamQuery {
    pub session_id: Option<String>,
    /// Polling interval in seconds (default 1s, max 30s)
    pub interval_secs: Option<u64>,
}

/// `GET /api/hitl/stream?session_id=...&interval_secs=1` — SSE endpoint that
/// emits HITL events as they appear. Useful for eval scripts that need to
/// detect HITL prompts in real time.
pub async fn hitl_stream_handler(
    AxumState(state): AxumState<HitlApiState>,
    Query(query): Query<HitlStreamQuery>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let session_filter = query.session_id.clone();
    let interval_secs = query.interval_secs.unwrap_or(1).clamp(1, 30);
    let store = Arc::clone(&state.store);

    let event_stream = stream! {
        // Send initial snapshot
        let initial = store.pending(session_filter.as_deref()).await;
        let snapshot_payload = serde_json::json!({
            "event": "snapshot",
            "pending": initial,
            "count": initial.len(),
        }).to_string();
        yield Ok(Event::default().event("snapshot").data(snapshot_payload));

        // Track which IDs we've already emitted so we don't repeat
        let mut emitted: std::collections::HashSet<String> = initial
            .iter()
            .map(|p| p.request_id.clone())
            .collect();

        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        interval.tick().await; // first tick is immediate, skip it

        loop {
            interval.tick().await;
            let current = store.pending(session_filter.as_deref()).await;
            let current_ids: std::collections::HashSet<String> = current
                .iter()
                .map(|p| p.request_id.clone())
                .collect();

            // Emit new pending requests
            for req in &current {
                if !emitted.contains(&req.request_id) {
                    let payload = serde_json::json!({
                        "event": "hitl_request",
                        "request": req,
                    }).to_string();
                    yield Ok(Event::default().event("hitl_request").data(payload));
                    emitted.insert(req.request_id.clone());
                }
            }

            // Detect resolved (no longer pending) requests
            let resolved: Vec<String> = emitted
                .iter()
                .filter(|id| !current_ids.contains(*id))
                .cloned()
                .collect();
            for id in resolved {
                let payload = serde_json::json!({
                    "event": "hitl_resolved",
                    "request_id": id,
                }).to_string();
                yield Ok(Event::default().event("hitl_resolved").data(payload));
                emitted.remove(&id);
            }
        }
    };

    Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}
