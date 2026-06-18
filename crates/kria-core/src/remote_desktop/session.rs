//! Remote-desktop session state machine + manager.
//!
//! Transport-neutral: the lifecycle (HITL request→confirm, idle expiry, global
//! halt kill-switch, audit, single-session guard, startup reconcile) is
//! identical regardless of the capture/stream technology. The concrete capture
//! backend is behind [`DesktopBackend`] (currently the portal + WebRTC backend).

use std::sync::{Arc, Mutex};

use serde::Serialize;
use uuid::Uuid;

use super::backend::DesktopBackend;
use super::portal::PortalBackend;
use crate::config::RemoteDesktopConfig;
use crate::safety::audit::{DecidedBy, Decision};
use crate::safety::{AuditLogger, RiskLevel};

/// Lifecycle of a remote-desktop session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    PendingApproval,
    Active,
    Stopped,
    Expired,
}

#[derive(Debug, thiserror::Error)]
pub enum RemoteDesktopError {
    #[error("remote desktop is disabled")]
    Disabled,
    #[error("no pending session with that id")]
    NoPendingSession,
    #[error("no active session")]
    NoActiveSession,
    #[error("a session is already active")]
    AlreadyActive,
    #[error("global halt engaged — remote desktop refused")]
    Halted,
    #[error("backend error: {0}")]
    Backend(String),
}

/// Public status snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteSessionStatus {
    pub state: SessionState,
    pub session_id: Option<String>,
    pub started_at: Option<i64>,
    pub last_activity: Option<i64>,
    pub idle_timeout_secs: i64,
    pub running: bool,
    pub backend: String,
}

/// Returned to the authenticated client on confirm. The capture session is now
/// live; the client opens the token-gated signaling WebSocket with this
/// `session_id` to negotiate the WebRTC stream.
#[derive(Debug, Clone, Serialize)]
pub struct SessionActivation {
    pub session_id: String,
}

struct Inner {
    state: SessionState,
    session_id: Option<String>,
    started_at: Option<i64>,
    last_activity: Option<i64>,
}

/// Manages the single live remote-desktop session.
pub struct RemoteDesktopManager {
    config: RemoteDesktopConfig,
    backend: Arc<dyn DesktopBackend>,
    audit: Option<Arc<AuditLogger>>,
    inner: Mutex<Inner>,
}

const AUDIT_SESSION: &str = "remote_desktop";

impl RemoteDesktopManager {
    /// Build with the real portal + WebRTC capture backend.
    pub fn new(config: RemoteDesktopConfig, audit: Option<Arc<AuditLogger>>) -> Self {
        let backend: Arc<dyn DesktopBackend> = Arc::new(PortalBackend::new(config.clone()));
        Self::with_backend(config, backend, audit)
    }

    /// Build with a custom backend (tests).
    pub fn with_backend(
        config: RemoteDesktopConfig,
        backend: Arc<dyn DesktopBackend>,
        audit: Option<Arc<AuditLogger>>,
    ) -> Self {
        Self {
            config,
            backend,
            audit,
            inner: Mutex::new(Inner {
                state: SessionState::Idle,
                session_id: None,
                started_at: None,
                last_activity: None,
            }),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Stream tuning for the WebRTC pipeline: (max_dimension, max_fps, encoder).
    pub fn stream_config(&self) -> (u32, u32, String) {
        (
            self.config.max_dimension,
            self.config.max_fps,
            self.config.video_encoder.clone(),
        )
    }

    /// Reconcile on startup/shutdown: if no active session, ensure the backend
    /// is disabled (so capture is never left running after a crash).
    pub fn reconcile_disabled(&self) {
        let active = matches!(self.inner.lock().unwrap().state, SessionState::Active);
        if !active {
            self.backend.disable();
        }
    }

    fn audit(&self, action: &str, decision: Decision, params: serde_json::Value) {
        if let Some(a) = self.audit.as_ref() {
            a.log(
                AUDIT_SESSION,
                action,
                &params,
                RiskLevel::Red,
                decision,
                DecidedBy::UserGui,
            );
        }
    }

    /// Step 1: request a session (arms a pending approval; does NOT capture).
    pub fn request(&self) -> Result<String, RemoteDesktopError> {
        if !self.config.enabled {
            return Err(RemoteDesktopError::Disabled);
        }
        if crate::safety::is_halted() {
            return Err(RemoteDesktopError::Halted);
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.state == SessionState::Active {
            return Err(RemoteDesktopError::AlreadyActive);
        }
        let id = Uuid::new_v4().to_string();
        inner.state = SessionState::PendingApproval;
        inner.session_id = Some(id.clone());
        self.audit(
            "remote_desktop_request",
            Decision::Approved,
            serde_json::json!({ "session_id": id }),
        );
        Ok(id)
    }

    /// Step 2: confirm (explicit HITL) — acquires the capture/input portal
    /// session and marks the session active.
    pub fn confirm(&self, session_id: &str) -> Result<SessionActivation, RemoteDesktopError> {
        if crate::safety::is_halted() {
            return Err(RemoteDesktopError::Halted);
        }
        {
            let inner = self.inner.lock().unwrap();
            match (&inner.state, &inner.session_id) {
                (SessionState::PendingApproval, Some(id)) if id == session_id => {}
                _ => return Err(RemoteDesktopError::NoPendingSession),
            }
        }

        self.backend.enable().map_err(RemoteDesktopError::Backend)?;

        let now = now();
        let mut inner = self.inner.lock().unwrap();
        inner.state = SessionState::Active;
        inner.started_at = Some(now);
        inner.last_activity = Some(now);
        drop(inner);

        self.audit(
            "remote_desktop_start",
            Decision::Approved,
            serde_json::json!({ "session_id": session_id }),
        );

        Ok(SessionActivation {
            session_id: session_id.to_string(),
        })
    }

    /// Stop / kill switch: release capture + tear down (idempotent).
    pub fn stop(&self) {
        let mut inner = self.inner.lock().unwrap();
        let was_active = inner.state == SessionState::Active;
        let sid = inner.session_id.clone();
        self.backend.disable();
        inner.state = SessionState::Stopped;
        inner.session_id = None;
        inner.started_at = None;
        inner.last_activity = None;
        drop(inner);
        if was_active {
            self.audit(
                "remote_desktop_stop",
                Decision::Approved,
                serde_json::json!({ "session_id": sid }),
            );
        }
    }

    /// Whether the signaling layer may currently stream (active + not halted +
    /// backend up).
    pub fn relay_allowed(&self) -> bool {
        if crate::safety::is_halted() {
            return false;
        }
        let inner = self.inner.lock().unwrap();
        inner.state == SessionState::Active && self.backend.is_running()
    }

    pub fn validate_session(&self, session_id: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.state == SessionState::Active && inner.session_id.as_deref() == Some(session_id)
    }

    pub fn touch(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.state == SessionState::Active {
            inner.last_activity = Some(now());
        }
    }

    pub fn audit_relay(&self, connected: bool, device_id: Option<&str>) {
        self.audit(
            if connected {
                "remote_desktop_connect"
            } else {
                "remote_desktop_disconnect"
            },
            Decision::Approved,
            serde_json::json!({ "device_id": device_id }),
        );
    }

    /// Expire on idle or global halt. Returns true if it tore the session down.
    pub fn enforce_idle(&self) -> bool {
        if crate::safety::is_halted() {
            self.stop();
            return true;
        }
        let should_expire = {
            let inner = self.inner.lock().unwrap();
            match (inner.state, inner.last_activity) {
                (SessionState::Active, Some(last)) if self.config.idle_timeout_secs > 0 => {
                    now() - last >= self.config.idle_timeout_secs
                }
                _ => false,
            }
        };
        if should_expire {
            let sid = self.inner.lock().unwrap().session_id.clone();
            self.backend.disable();
            let mut inner = self.inner.lock().unwrap();
            inner.state = SessionState::Expired;
            inner.session_id = None;
            inner.started_at = None;
            inner.last_activity = None;
            drop(inner);
            self.audit(
                "remote_desktop_expire",
                Decision::Approved,
                serde_json::json!({ "session_id": sid }),
            );
            return true;
        }
        false
    }

    pub fn status(&self) -> RemoteSessionStatus {
        let inner = self.inner.lock().unwrap();
        RemoteSessionStatus {
            state: inner.state,
            session_id: inner.session_id.clone(),
            started_at: inner.started_at,
            last_activity: inner.last_activity,
            idle_timeout_secs: self.config.idle_timeout_secs,
            running: self.backend.is_running(),
            backend: self.backend.label(),
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct FakeBackend {
        running: AtomicBool,
        enables: AtomicUsize,
        disables: AtomicUsize,
    }
    impl DesktopBackend for FakeBackend {
        fn enable(&self) -> Result<(), String> {
            self.running.store(true, Ordering::SeqCst);
            self.enables.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn disable(&self) {
            self.running.store(false, Ordering::SeqCst);
            self.disables.fetch_add(1, Ordering::SeqCst);
        }
        fn is_running(&self) -> bool {
            self.running.load(Ordering::SeqCst)
        }
        fn label(&self) -> String {
            "fake".into()
        }
    }

    fn manager(enabled: bool) -> (Arc<FakeBackend>, RemoteDesktopManager) {
        let cfg = RemoteDesktopConfig {
            enabled,
            idle_timeout_secs: 300,
            ..Default::default()
        };
        let backend = Arc::new(FakeBackend::default());
        let mgr = RemoteDesktopManager::with_backend(cfg, backend.clone(), None);
        (backend, mgr)
    }

    #[test]
    fn disabled_refuses_request() {
        let (_b, mgr) = manager(false);
        assert!(matches!(mgr.request(), Err(RemoteDesktopError::Disabled)));
    }

    #[test]
    fn request_confirm_enables_backend() {
        let (backend, mgr) = manager(true);
        let id = mgr.request().unwrap();
        assert_eq!(mgr.status().state, SessionState::PendingApproval);
        let act = mgr.confirm(&id).unwrap();
        assert_eq!(act.session_id, id);
        assert_eq!(mgr.status().state, SessionState::Active);
        assert!(backend.is_running());
        assert!(mgr.relay_allowed());
        assert!(mgr.validate_session(&id));
    }

    #[test]
    fn confirm_requires_matching_pending_id() {
        let (_b, mgr) = manager(true);
        mgr.request().unwrap();
        assert!(matches!(
            mgr.confirm("wrong"),
            Err(RemoteDesktopError::NoPendingSession)
        ));
    }

    #[test]
    fn stop_disables_backend() {
        let (backend, mgr) = manager(true);
        let id = mgr.request().unwrap();
        mgr.confirm(&id).unwrap();
        mgr.stop();
        assert_eq!(mgr.status().state, SessionState::Stopped);
        assert!(!backend.is_running());
        assert!(!mgr.relay_allowed());
    }

    #[test]
    fn idle_expiry_disables_backend() {
        let (backend, mgr) = manager(true);
        let id = mgr.request().unwrap();
        mgr.confirm(&id).unwrap();
        {
            let mut inner = mgr.inner.lock().unwrap();
            inner.last_activity = Some(now() - 10_000);
        }
        assert!(mgr.enforce_idle());
        assert_eq!(mgr.status().state, SessionState::Expired);
        assert!(!backend.is_running());
    }

    #[test]
    fn double_activate_blocked() {
        let (_b, mgr) = manager(true);
        let id = mgr.request().unwrap();
        mgr.confirm(&id).unwrap();
        assert!(matches!(
            mgr.request(),
            Err(RemoteDesktopError::AlreadyActive)
        ));
    }

    #[test]
    fn reconcile_disables_when_not_active() {
        let (backend, mgr) = manager(true);
        backend.enable().unwrap();
        assert!(backend.is_running());
        mgr.reconcile_disabled();
        assert!(!backend.is_running());
    }
}
