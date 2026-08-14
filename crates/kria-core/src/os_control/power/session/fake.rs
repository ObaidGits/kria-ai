//! Deny-live fake for the power **session** transport (Task 3.8, design §9.7).
//!
//! # Why a session fake is not a trivial stub
//!
//! Session operations end the user's session: lock, sleep, hibernate, logout,
//! shutdown. Two properties make them different from an ordinary mutation, and
//! this fake exists to make both testable:
//!
//! 1. **They are `Accepted`, not `Applied`.** Once `loginctl suspend` is
//!    accepted, observability terminates — the machine goes to sleep and there is
//!    nobody left to re-observe. A fake that returned `Applied` would let a suite
//!    "prove" a verification that can never happen on real hardware, so the
//!    outcome is scripted per test rather than assumed.
//! 2. **A capability probe gates them.** Hibernate is unavailable on many hosts.
//!    `Ok(false)` and `Err(_)` must both prevent dispatch, and this fake can
//!    script each independently.
//!
//! Nothing here opens a transport: [`deny_live_transport`] is never called
//! because no real session manager is ever contacted, which is exactly why the
//! deny-live suites can drive it.

use std::sync::Mutex;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    StructuredCommandRequest, StructuredCommandSummary,
};
use crate::os_control::receipt::ApplyOutcome;

use super::selection::{PowerSessionBackend, ScheduledShutdown};
use super::PowerSessionTransport;

/// A scriptable, in-memory session transport.
pub struct FakePowerSessionTransport {
    backend: PowerSessionBackend,
    /// An **ordered queue** of lock-state answers, consumed one per read.
    ///
    /// A queue rather than a single value because one governed mutation performs
    /// several reads — pre-observation, under-lease re-observation, post-apply
    /// re-observation, then independent verification — and a lock lifecycle is
    /// only meaningful if those reads can differ. A fake returning one fixed
    /// value would make every lock look already-locked (`Unchanged`).
    locked: Mutex<std::collections::VecDeque<bool>>,
    /// `Some(available)` scripts the probe; `None` makes it fail.
    hibernate: Mutex<Option<bool>>,
    /// The caller's own session id, when scripted.
    session_id: Mutex<Option<String>>,
    /// `Some(Some(s))` = a shutdown is pending, `Some(None)` = positively none,
    /// `None` = the schedule could not be read (unknown, not absent).
    scheduled: Mutex<Option<Option<ScheduledShutdown>>>,
    /// The outcome `dispatch` returns. Defaults to `Applied` for the lock-style
    /// operations that really are verifiable.
    outcome: Mutex<Option<ApplyOutcome>>,
    captured: Mutex<Vec<StructuredCommandSummary>>,
}

impl FakePowerSessionTransport {
    /// A fake over `backend`, with no lock state and no hibernate answer
    /// scripted — both fail until a test says otherwise, so a suite cannot
    /// accidentally depend on a fabricated default.
    #[must_use]
    pub fn new(backend: PowerSessionBackend) -> Self {
        Self {
            backend,
            locked: Mutex::new(std::collections::VecDeque::new()),
            hibernate: Mutex::new(None),
            session_id: Mutex::new(None),
            scheduled: Mutex::new(None),
            outcome: Mutex::new(None),
            captured: Mutex::new(Vec::new()),
        }
    }

    /// Append one lock-state answer to the read queue. Call it once per read the
    /// test expects, in order.
    #[must_use]
    pub fn locked_ok(self, locked: bool) -> Self {
        self.locked.lock().expect("locked mutex").push_back(locked);
        self
    }

    /// Script the hibernate capability probe.
    #[must_use]
    pub fn hibernate_available(self, available: bool) -> Self {
        *self.hibernate.lock().expect("hibernate mutex") = Some(available);
        self
    }

    /// Script the caller's own session id.
    #[must_use]
    pub fn current_session_id(self, id: impl Into<String>) -> Self {
        *self.session_id.lock().expect("session id mutex") = Some(id.into());
        self
    }

    /// Script the pending-shutdown read. `None` means "positively nothing
    /// scheduled"; leaving this unset makes the read fail as *unknown*.
    #[must_use]
    pub fn scheduled_shutdown(self, pending: Option<ScheduledShutdown>) -> Self {
        *self.scheduled.lock().expect("scheduled mutex") = Some(pending);
        self
    }

    /// Script the outcome `dispatch` returns — `Accepted` for session-ending
    /// operations whose effect can never be re-observed.
    #[must_use]
    pub fn dispatch_outcome(self, outcome: ApplyOutcome) -> Self {
        *self.outcome.lock().expect("outcome mutex") = Some(outcome);
        self
    }

    /// How many times `dispatch` ran. A governed mutation must apply exactly once.
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        self.captured.lock().expect("captured mutex").len()
    }

    /// The redacted projections of every dispatched command, in order.
    #[must_use]
    pub fn captured(&self) -> Vec<StructuredCommandSummary> {
        self.captured.lock().expect("captured mutex").clone()
    }

    fn unreadable(&self, what: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: SafeText::new(format!(
                "scripted session fake has no {what} answer; it is unknown, not a default"
            )),
            retryable: true,
        }
    }
}

#[async_trait::async_trait]
impl PowerSessionTransport for FakePowerSessionTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(format!("fake-session-{}", self.backend.as_str()))
    }

    fn selected_backend(&self) -> PowerSessionBackend {
        self.backend
    }

    async fn read_locked(&self, _ctx: &HostExecutionContext) -> Result<bool, OsControlError> {
        // An exhausted queue is an *unknown* lock state, never a default. If a
        // test scripted fewer reads than the governed path performs, that is a
        // fact worth failing on rather than papering over.
        self.locked
            .lock()
            .expect("locked mutex")
            .pop_front()
            .ok_or_else(|| self.unreadable("lock-state"))
    }

    async fn hibernate_available(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<bool, OsControlError> {
        self.hibernate
            .lock()
            .expect("hibernate mutex")
            .ok_or_else(|| self.unreadable("hibernate-capability"))
    }

    async fn read_current_session_id(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<String, OsControlError> {
        self.session_id
            .lock()
            .expect("session id mutex")
            .clone()
            // Refusing is the right default: guessing a session id could
            // terminate someone else's session.
            .ok_or_else(|| self.unreadable("session-id"))
    }

    async fn read_scheduled_shutdown(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<Option<ScheduledShutdown>, OsControlError> {
        self.scheduled
            .lock()
            .expect("scheduled mutex")
            .clone()
            .ok_or_else(|| self.unreadable("pending-shutdown"))
    }

    async fn dispatch(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Record the redacted projection, never the raw argv, so a suite asserts
        // on exactly what an audit record would show.
        self.captured
            .lock()
            .expect("captured mutex")
            .push(request.safe_summary());

        // An unscripted dispatch is a DENIED/absent transport, not a success.
        // Returning a default `Applied` here would let a suite "prove" an effect
        // the transport never accepted — and would hide the very case this fake
        // exists to test: a Polkit denial must stay denied, with no retry and no
        // privileged fallback.
        self.outcome
            .lock()
            .expect("outcome mutex")
            .clone()
            .ok_or_else(|| self.unreadable("dispatch"))
    }
}
