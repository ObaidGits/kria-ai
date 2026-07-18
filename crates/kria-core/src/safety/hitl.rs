use crate::safety::RiskLevel;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::Mutex;

/// Represents a pending HITL approval request.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub action: String,
    pub parameters: serde_json::Value,
    pub risk_level: RiskLevel,
    pub description: String,
    pub timeout_seconds: u64,
    pub rollback_available: bool,
}

/// User response to an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum ApprovalResponse {
    Approved,
    Denied,
    Timeout,
}

/// Internal pending request with its response channel.
struct PendingRequest {
    request: ApprovalRequest,
    responder: oneshot::Sender<ApprovalResponse>,
}

/// Human-In-The-Loop gateway. All RED actions pass through here.
///
/// The gateway presents the request to the user (via GUI/voice/API)
/// and waits for a response within the configured timeout.
pub struct HitlGateway {
    pending: Arc<Mutex<HashMap<String, PendingRequest>>>,
    /// Channel to notify frontends of new approval requests.
    request_tx: mpsc::UnboundedSender<ApprovalRequest>,
    request_rx: Arc<Mutex<mpsc::UnboundedReceiver<ApprovalRequest>>>,
    default_timeout: Duration,
}

impl HitlGateway {
    pub fn new(default_timeout_secs: u64) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            request_tx: tx,
            request_rx: Arc::new(Mutex::new(rx)),
            default_timeout: Duration::from_secs(default_timeout_secs),
        }
    }

    /// Generate a unique request ID for HITL approval.
    /// Call this before `request_approval_with_id` so the ID can be sent to
    /// the frontend before the gateway blocks.
    pub fn generate_request_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Register a pending request before any UI event is emitted. Callers that
    /// own presentation use this to avoid a race where an immediate response
    /// arrives before the request exists in the gateway.
    pub async fn prepare_approval_with_id(
        &self,
        request_id: &str,
        action: &str,
        parameters: serde_json::Value,
        risk_level: RiskLevel,
        description: &str,
        rollback_available: bool,
    ) -> oneshot::Receiver<ApprovalResponse> {
        let request = ApprovalRequest {
            id: request_id.to_string(),
            action: action.to_string(),
            parameters,
            risk_level,
            description: description.to_string(),
            timeout_seconds: self.default_timeout.as_secs(),
            rollback_available,
        };
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(
            request_id.to_string(),
            PendingRequest {
                request: request.clone(),
                responder: tx,
            },
        );
        let _ = self.request_tx.send(request);
        rx
    }

    /// Await a response to a request registered by
    /// [`prepare_approval_with_id`](Self::prepare_approval_with_id).
    pub async fn await_prepared_approval(
        &self,
        request_id: &str,
        rx: oneshot::Receiver<ApprovalResponse>,
    ) -> ApprovalResponse {
        match tokio::time::timeout(self.default_timeout, rx).await {
            Ok(Ok(response)) => response,
            _ => {
                self.pending.lock().await.remove(request_id);
                tracing::warn!(request_id = %request_id, "HITL request timed out, auto-denying");
                ApprovalResponse::Timeout
            }
        }
    }

    /// Submit a RED action for approval using a pre-generated request ID.
    /// Blocks until the user responds or timeout.
    pub async fn request_approval_with_id(
        &self,
        request_id: &str,
        action: &str,
        parameters: serde_json::Value,
        risk_level: RiskLevel,
        description: &str,
        rollback_available: bool,
    ) -> ApprovalResponse {
        let rx = self
            .prepare_approval_with_id(
                request_id,
                action,
                parameters,
                risk_level,
                description,
                rollback_available,
            )
            .await;
        self.await_prepared_approval(request_id, rx).await
    }

    /// Submit a RED action for approval. Blocks until the user responds or timeout.
    /// Generates a new UUID internally — prefer `request_approval_with_id` when
    /// you need the ID before calling (e.g. to send it to the frontend first).
    pub async fn request_approval(
        &self,
        action: &str,
        parameters: serde_json::Value,
        risk_level: RiskLevel,
        description: &str,
        rollback_available: bool,
    ) -> ApprovalResponse {
        let id = Self::generate_request_id();
        self.request_approval_with_id(
            &id,
            action,
            parameters,
            risk_level,
            description,
            rollback_available,
        )
        .await
    }

    /// Respond to a pending request (called by GUI/voice handler).
    pub async fn respond(&self, request_id: &str, response: ApprovalResponse) -> bool {
        let mut pending = self.pending.lock().await;
        if let Some(req) = pending.remove(request_id) {
            let _ = req.responder.send(response);
            true
        } else {
            false
        }
    }

    /// Subscribe to new approval request notifications.
    pub fn subscribe(&self) -> &Arc<Mutex<mpsc::UnboundedReceiver<ApprovalRequest>>> {
        &self.request_rx
    }

    /// Get all currently pending requests.
    pub async fn pending_requests(&self) -> Vec<ApprovalRequest> {
        let pending = self.pending.lock().await;
        pending.values().map(|p| p.request.clone()).collect()
    }

    /// Cancel all pending requests (emergency stop).
    pub async fn cancel_all(&self) {
        let mut pending = self.pending.lock().await;
        for (_, req) in pending.drain() {
            let _ = req.responder.send(ApprovalResponse::Denied);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prepared_request_accepts_immediate_response_without_race() {
        let gateway = HitlGateway::new(1);
        let request_id = HitlGateway::generate_request_id();
        let rx = gateway
            .prepare_approval_with_id(
                &request_id,
                "config_patch",
                serde_json::json!({ "section": "ui", "field": "theme" }),
                RiskLevel::Yellow,
                "change theme",
                false,
            )
            .await;

        assert!(
            gateway
                .respond(&request_id, ApprovalResponse::Approved)
                .await
        );
        assert_eq!(
            gateway.await_prepared_approval(&request_id, rx).await,
            ApprovalResponse::Approved
        );
        assert!(gateway.pending_requests().await.is_empty());
    }
}
