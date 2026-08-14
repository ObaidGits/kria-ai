//! Deny-live fake automation transport (Task 0.4 / OSC-033) — test composition
//! only.
//!
//! The fake never launches a process and never touches the host. It keeps an
//! in-memory task table, records every dispatched argv so a test can assert the
//! exact command shape, and applies an `enable`/`disable` dispatch to its own
//! table so the governed verification step reads a fact the mutation actually
//! produced rather than one the fake asserted up front.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::automation::{
    AutomationBackend, AutomationListing, AutomationTaskState, AutomationTransport,
    AUTOMATION_PROVIDER_ID,
};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::StructuredCommandRequest;
use crate::os_control::receipt::{AppliedDispatch, ApplyOutcome};

/// The in-memory state a fake task carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeTask {
    /// Whether the task is enabled. `None` models a static unit with no
    /// enablement state.
    pub enabled: Option<bool>,
    /// The configuration revision, bumped by a successful dispatch.
    pub revision: Option<u64>,
    /// The next run instant, if any.
    pub next_run_ms: Option<u64>,
}

impl FakeTask {
    /// An enabled task at a given revision.
    #[must_use]
    pub fn enabled(revision: u64) -> Self {
        Self {
            enabled: Some(true),
            revision: Some(revision),
            next_run_ms: Some(1_700_000_000_000),
        }
    }

    /// A disabled task at a given revision.
    #[must_use]
    pub fn disabled(revision: u64) -> Self {
        Self {
            enabled: Some(false),
            revision: Some(revision),
            next_run_ms: None,
        }
    }

    /// A static unit: present, but with no enablement state at all.
    #[must_use]
    pub fn static_unit(revision: u64) -> Self {
        Self {
            enabled: None,
            revision: Some(revision),
            next_run_ms: None,
        }
    }
}

#[derive(Debug, Default)]
struct FakeState {
    tasks: BTreeMap<String, FakeTask>,
    dispatched: Vec<Vec<String>>,
    /// When set, every read fails with this reason — used to prove the domain
    /// fails closed instead of reporting a default.
    read_failure: Option<String>,
    /// When set, the next dispatch fails.
    dispatch_failure: Option<String>,
}

/// A fake automation transport.
pub struct FakeAutomationTransport {
    backend: AutomationBackend,
    state: Mutex<FakeState>,
}

impl FakeAutomationTransport {
    /// A fake over the systemd-user-timer backend with no tasks.
    #[must_use]
    pub fn new() -> Self {
        Self {
            backend: AutomationBackend::SystemdUserTimers,
            state: Mutex::new(FakeState::default()),
        }
    }

    /// A fake over the crontab backend (which cannot serve a modification).
    #[must_use]
    pub fn crontab() -> Self {
        Self {
            backend: AutomationBackend::Crontab,
            state: Mutex::new(FakeState::default()),
        }
    }

    /// Seed a task.
    #[must_use]
    pub fn with_task(self, unit: &str, task: FakeTask) -> Self {
        self.state
            .lock()
            .expect("fake state poisoned")
            .tasks
            .insert(unit.to_string(), task);
        self
    }

    /// Make every read fail, so a test can prove the domain never fabricates an
    /// observation.
    #[must_use]
    pub fn with_read_failure(self, reason: &str) -> Self {
        self.state
            .lock()
            .expect("fake state poisoned")
            .read_failure = Some(reason.to_string());
        self
    }

    /// Make the next dispatch fail.
    #[must_use]
    pub fn with_dispatch_failure(self, reason: &str) -> Self {
        self.state
            .lock()
            .expect("fake state poisoned")
            .dispatch_failure = Some(reason.to_string());
        self
    }

    /// Every argv dispatched so far.
    #[must_use]
    pub fn dispatched(&self) -> Vec<Vec<String>> {
        self.state
            .lock()
            .expect("fake state poisoned")
            .dispatched
            .clone()
    }

    /// The current state of a seeded task.
    #[must_use]
    pub fn task(&self, unit: &str) -> Option<FakeTask> {
        self.state
            .lock()
            .expect("fake state poisoned")
            .tasks
            .get(unit)
            .cloned()
    }
}

impl Default for FakeAutomationTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AutomationTransport for FakeAutomationTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(format!("fake-{AUTOMATION_PROVIDER_ID}"))
    }

    fn selected_backend(&self) -> AutomationBackend {
        self.backend
    }

    async fn read_listing(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<AutomationListing, OsControlError> {
        let state = self.state.lock().expect("fake state poisoned");
        if let Some(reason) = &state.read_failure {
            return Err(OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(reason.clone()),
                retryable: false,
            });
        }
        Ok(AutomationListing {
            cron_jobs: String::new(),
            systemd_timers: state
                .tasks
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }

    async fn read_task(
        &self,
        _ctx: &HostExecutionContext,
        unit: &str,
    ) -> Result<AutomationTaskState, OsControlError> {
        let state = self.state.lock().expect("fake state poisoned");
        if let Some(reason) = &state.read_failure {
            // A failed read is an error, never an absent-looking state.
            return Err(OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(reason.clone()),
                retryable: false,
            });
        }
        Ok(match state.tasks.get(unit) {
            Some(task) => AutomationTaskState {
                present: true,
                enabled: task.enabled,
                revision: task.revision,
                next_run_ms: task.next_run_ms,
            },
            None => AutomationTaskState {
                present: false,
                enabled: None,
                revision: None,
                next_run_ms: None,
            },
        })
    }

    async fn dispatch(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError> {
        let args = request.args().to_vec();
        let mut state = self.state.lock().expect("fake state poisoned");
        state.dispatched.push(args.clone());
        if let Some(reason) = state.dispatch_failure.take() {
            return Err(OsControlError::Unavailable {
                provider: Some(ProviderId::new(format!("fake-{AUTOMATION_PROVIDER_ID}"))),
                reason: SafeText::new(reason),
                retryable: false,
            });
        }

        // Apply the effect the argv actually asks for, so verification reads a
        // fact this dispatch produced.
        if let [_user, verb, unit] = args.as_slice() {
            let enabled = match verb.as_str() {
                "enable" => Some(true),
                "disable" => Some(false),
                _ => None,
            };
            if let Some(enabled) = enabled {
                if let Some(task) = state.tasks.get_mut(unit) {
                    task.enabled = Some(enabled);
                    task.revision = task.revision.map(|r| r + 1);
                }
            }
        }
        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            None,
            BoundedVec::new(),
        )))
    }
}
