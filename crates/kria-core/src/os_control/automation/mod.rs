//! Automation domain: the listing slice plus the typed mutation slice of
//! `AutomationControl` (design §3, §9.13).
//!
//! linux-os-control-production **Task 2.5** (`list_scheduled_tasks`) and
//! **Task 4.5** (`modify_scheduled_task`, `list_workflows`, `run_workflow`) —
//! OSC-027, OSC-028.
//!
//! # Why nothing here can schedule a command
//!
//! The deleted `create_scheduled_task`/`delete_scheduled_task` handlers spawned
//! `crontab` directly with a caller-supplied command line: no policy, no grant,
//! no lease, no audit, no verification, and a persistent effect that outlived
//! the session. This module is the typed replacement, and the property that
//! makes it safe is structural rather than procedural:
//!
//! * a schedule is a [`typed::TypedSchedule`] — three closed shapes, bounded
//!   fields, no expression language;
//! * an action is a [`typed::CanonicalAction`] — an `os.<tool>` operation that
//!   must already exist in the frozen manifest, with parameters bound by digest.
//!   A shell string is not a valid operation id, so "run this later" is not a
//!   representable state;
//! * a workflow is code in [`workflows::IN_TREE_WORKFLOWS`], never a body a
//!   caller supplies, and it is addressed by stable id only;
//! * every mutation goes through [`crate::os_control::governed`] with the
//!   caller's `expected_revision` checked against a freshly read provider
//!   revision immediately before the change.
//!
//! # What this build cannot do, and says so
//!
//! Only the systemd-user-timer backend has a stable per-task identity and an
//! observable revision, so it is the only backend that can serve a
//! modification; `crontab` is listable but explicitly refused for mutation.
//! Within that backend, only the `enabled` facet of a patch can be applied and
//! verified — rewriting a timer's schedule or its contained action needs the
//! governed unit-file writer, which is not composed. Those patches are refused
//! with a precise reason rather than approximated by editing text.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    BoundedVec, CapabilityId, ComparatorKind, DesiredStateControl, Digest, NonEmptyBoundedVec,
    OsEvidenceSource, ProviderId, SafeErrorCode, SafeField, SafeStepId, SafeText,
    VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest,
};
use crate::os_control::receipt::{
    ApplyOutcome, PartialEffectCause, RedactedObservation, RollbackToken, SatisfyingVerification,
    UncertainDispatch, UncertainEffectCause, VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;
use crate::safety::RiskLevel;

pub mod selection;
pub mod typed;
pub mod workflows;

/// Deny-live fake transport (Task 0.4 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;

pub use selection::{AutomationBackend, TimerShow, UnitEnablement};
pub use typed::{
    AutomationId, CanonicalAction, Revision, TypedAutomationPatch, TypedSchedule, Weekday,
    WorkflowId,
};
pub use workflows::{WorkflowDescriptor, WorkflowPage, WorkflowStep, WorkflowStepAction};

/// The stable provider identity for the cron/systemd-timer backend.
pub const AUTOMATION_PROVIDER_ID: &str = "automation-cron-systemd-timers";

/// A normalized listing of the current KRIA-independent cron jobs + systemd
/// user timers (design §5, §9.13). Retained as raw trimmed text (matching the
/// pre-migration `cron_jobs`/`systemd_timers` result fields exactly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationListing {
    /// The current user's crontab text (trimmed; empty if none/unavailable).
    pub cron_jobs: String,
    /// The current user's `systemctl --user list-timers` text (trimmed).
    pub systemd_timers: String,
}

impl NormalizedObservation for AutomationListing {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "automation:{}:{}",
            self.cron_jobs, self.systemd_timers
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Typed observations
// ─────────────────────────────────────────────────────────────────────────────

/// Which fact an automation observation is about.
///
/// Part of the observation digest, so a task-configuration fact can never
/// satisfy a workflow-run postcondition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationFocus {
    /// A scheduled task's configuration (`modify_scheduled_task`).
    TaskConfiguration,
    /// A workflow run's progress (`run_workflow`).
    WorkflowRun,
}

impl AutomationFocus {
    /// The stable snake_case token used in the digest.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AutomationFocus::TaskConfiguration => "task_configuration",
            AutomationFocus::WorkflowRun => "workflow_run",
        }
    }
}

/// The current state of one scheduled task, as read from the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationTaskState {
    /// Whether the provider knows this task at all. `false` is the **absent**
    /// fact and is never produced from a failed read.
    pub present: bool,
    /// Whether the task is enabled, when the backend expresses enablement.
    /// `None` means the backend has no enablement for this unit (a static or
    /// generated unit) — distinct from "disabled".
    pub enabled: Option<bool>,
    /// The provider's configuration revision, when one could be read.
    pub revision: Option<Revision>,
    /// The next scheduled run, Unix epoch milliseconds, when the provider
    /// reported one.
    pub next_run_ms: Option<u64>,
}

/// A focused automation observation.
///
/// `revision` and `next_run_ms` are **reported but not part of the digest**: a
/// revision is a precondition (checked with the caller's `expected_revision`
/// immediately before the mutation), not a postcondition. Binding it into the
/// postcondition digest would make every successful patch verify as
/// contradicted, because the provider bumps the revision as a result of the
/// change itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationObservation {
    /// The fact this observation is about.
    pub focus: AutomationFocus,
    /// The stable identity the fact belongs to (task or workflow id).
    pub identity: String,
    /// Whether the target exists.
    pub present: bool,
    /// Enablement, for a task-configuration fact.
    pub enabled: Option<bool>,
    /// The schedule digest, when the backend can express it.
    pub schedule_digest: Option<Digest>,
    /// The contained action digest, when the backend can express it.
    pub action_digest: Option<Digest>,
    /// Steps completed, for a workflow-run fact.
    pub completed_step_count: Option<u32>,
    /// The step that failed, for a workflow-run fact.
    pub failed_step: Option<String>,
    /// The provider revision (reported only; excluded from the digest).
    pub revision: Option<Revision>,
    /// The next run instant (reported only; excluded from the digest).
    pub next_run_ms: Option<u64>,
}

impl AutomationObservation {
    /// A task-configuration fact.
    #[must_use]
    pub fn task_configuration(
        identity: impl Into<String>,
        present: bool,
        enabled: Option<bool>,
    ) -> Self {
        Self {
            focus: AutomationFocus::TaskConfiguration,
            identity: identity.into(),
            present,
            enabled,
            schedule_digest: None,
            action_digest: None,
            completed_step_count: None,
            failed_step: None,
            revision: None,
            next_run_ms: None,
        }
    }

    /// A workflow-run fact.
    #[must_use]
    pub fn workflow_run(
        identity: impl Into<String>,
        completed_step_count: u32,
        failed_step: Option<String>,
    ) -> Self {
        Self {
            focus: AutomationFocus::WorkflowRun,
            identity: identity.into(),
            present: true,
            enabled: None,
            schedule_digest: None,
            action_digest: None,
            completed_step_count: Some(completed_step_count),
            failed_step,
            revision: None,
            next_run_ms: None,
        }
    }

    /// Attach the reported revision / next-run facts (not digest-bound).
    #[must_use]
    pub fn with_reported(mut self, revision: Option<Revision>, next_run_ms: Option<u64>) -> Self {
        self.revision = revision;
        self.next_run_ms = next_run_ms;
        self
    }
}

impl NormalizedObservation for AutomationObservation {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "automation:{}:{}:{}:{}:{}:{}:{}:{}",
            self.focus.as_str(),
            self.identity,
            self.present,
            self.enabled
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            self.schedule_digest
                .as_ref()
                .map(|d| d.as_hex().to_string())
                .unwrap_or_else(|| "unbound".to_string()),
            self.action_digest
                .as_ref()
                .map(|d| d.as_hex().to_string())
                .unwrap_or_else(|| "unbound".to_string()),
            self.completed_step_count
                .map(|c| c.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            self.failed_step.as_deref().unwrap_or("none"),
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Typed requests
// ─────────────────────────────────────────────────────────────────────────────

/// The concrete typed automation operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationOp {
    /// Patch an existing scheduled task (`modify_scheduled_task`).
    UpdateTask {
        /// The task's stable identity.
        task_id: AutomationId,
        /// The revision the caller read before deciding.
        expected_revision: Revision,
        /// The validated typed patch.
        patch: TypedAutomationPatch,
    },
    /// Run a reviewed in-tree workflow (`run_workflow`).
    RunWorkflow {
        /// The workflow's stable identity.
        workflow_id: WorkflowId,
        /// The workflow definition revision the caller read.
        expected_revision: Revision,
    },
}

impl AutomationOp {
    /// The canonical tool name this operation maps to.
    #[must_use]
    pub const fn action_name(&self) -> &'static str {
        match self {
            AutomationOp::UpdateTask { .. } => "modify_scheduled_task",
            AutomationOp::RunWorkflow { .. } => "run_workflow",
        }
    }

    /// The stable identity this operation targets.
    #[must_use]
    pub fn identity(&self) -> &str {
        match self {
            AutomationOp::UpdateTask { task_id, .. } => task_id.as_str(),
            AutomationOp::RunWorkflow { workflow_id, .. } => workflow_id.as_str(),
        }
    }
}

/// A fully-described typed automation request. Carries the canonical
/// `action`/`params` so the governed [`StructuredCommandRequest`] binds them
/// against the grant.
#[derive(Debug, Clone)]
pub struct AutomationRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The concrete typed operation.
    pub op: AutomationOp,
}

impl AutomationRequest {
    /// The fact this operation's postcondition concerns.
    #[must_use]
    pub const fn focus(&self) -> AutomationFocus {
        match self.op {
            AutomationOp::UpdateTask { .. } => AutomationFocus::TaskConfiguration,
            AutomationOp::RunWorkflow { .. } => AutomationFocus::WorkflowRun,
        }
    }

    /// The desired end state for this mutation.
    ///
    /// A patch asserts only what it changes: a patch that does not touch
    /// enablement leaves `enabled` unknown rather than asserting a value nobody
    /// asked for.
    #[must_use]
    pub fn desired_state(&self) -> AutomationObservation {
        match &self.op {
            AutomationOp::UpdateTask { task_id, patch, .. } => {
                AutomationObservation::task_configuration(
                    task_id.as_str(),
                    true,
                    patch.enabled,
                )
            }
            AutomationOp::RunWorkflow { workflow_id, .. } => {
                // Filled in by the provider, which knows the step count for the
                // named revision; the handler never invents it.
                AutomationObservation::workflow_run(workflow_id.as_str(), 0, None)
            }
        }
    }

    /// The idempotency/verification comparator. Every automation postcondition
    /// is an exact typed fact, so there is no tolerance.
    #[must_use]
    pub const fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

fn invalid(field: &str, reason: &str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new(field),
        reason: SafeText::new(reason),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw automation transport seam. The live implementation is a
/// deny-live-gated adapter over `systemctl --user` / `crontab -l` (structured,
/// no shell); deny-live tests inject [`fake::FakeAutomationTransport`].
#[async_trait]
pub trait AutomationTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// The selected backend.
    fn selected_backend(&self) -> AutomationBackend;

    /// Read the current cron jobs + systemd timers listing.
    async fn read_listing(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<AutomationListing, OsControlError>;

    /// Read one task's typed state by unit identity.
    ///
    /// `Ok(AutomationTaskState { present: false, .. })` is the unambiguous
    /// "there is no such task" fact. A read that could not be performed must
    /// return `Err`, never an absent-looking state.
    async fn read_task(
        &self,
        ctx: &HostExecutionContext,
        unit: &str,
    ) -> Result<AutomationTaskState, OsControlError>;

    /// Dispatch one governed structured command.
    async fn dispatch(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct RollbackSnapshot {
    before: AutomationObservation,
    action: String,
    params: serde_json::Value,
    unit: String,
}

/// The `AutomationControl` provider (design §3, §4, §9.13). Generic over the
/// [`AutomationTransport`] so the same governed logic runs over the live
/// adapter and the deny-live fake.
pub struct AutomationControl<T: AutomationTransport> {
    transport: T,
    policy: CommandPolicy,
    /// The closed workflow registry this provider will execute from. Production
    /// composition always uses [`workflows::IN_TREE_WORKFLOWS`]; a test
    /// composition may inject a fixture registry so the execution machinery is
    /// exercised without shipping a workflow that has not been reviewed.
    registry: &'static [WorkflowDescriptor],
    /// Prior-state snapshots keyed by session id, captured in `apply` for
    /// `rollback`.
    snapshots: Mutex<HashMap<String, RollbackSnapshot>>,
}

impl<T: AutomationTransport> AutomationControl<T> {
    /// Compose over a transport, using the shipped closed workflow registry.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            policy: CommandPolicy::new(),
            registry: workflows::IN_TREE_WORKFLOWS,
            snapshots: Mutex::new(HashMap::new()),
        }
    }

    /// Compose over a transport with a fixture workflow registry (tests only).
    #[cfg(feature = "os-control-test")]
    #[must_use]
    pub fn with_registry(transport: T, registry: &'static [WorkflowDescriptor]) -> Self {
        Self {
            transport,
            policy: CommandPolicy::new(),
            registry,
            snapshots: Mutex::new(HashMap::new()),
        }
    }

    /// Borrow the underlying transport (used by tests).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The provider identity.
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        self.transport.provider_id()
    }

    /// The selected backend.
    #[must_use]
    pub fn backend(&self) -> AutomationBackend {
        self.transport.selected_backend()
    }

    /// Read the current listing (`list_scheduled_tasks`; a pure read outside
    /// the mutation lifecycle).
    pub async fn list(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<AutomationListing, OsControlError> {
        self.transport.read_listing(ctx).await
    }

    /// One page of reviewed workflows (`list_workflows`; a pure read).
    ///
    /// Enumerates the closed in-tree registry. An empty page is the truthful
    /// answer when no workflow has been reviewed, and is distinguishable from a
    /// failure because a failure returns `Err`.
    pub fn list_workflows(
        &self,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<WorkflowPage, OsControlError> {
        workflows::page(self.registry, cursor, limit)
    }

    fn evidence_source(&self) -> OsEvidenceSource {
        OsEvidenceSource::StructuredCommandQuery
    }

    /// Resolve and validate the unit identity a task operation targets.
    fn unit_of(&self, task_id: &AutomationId) -> Result<String, OsControlError> {
        if !self.backend().supports_modification() {
            return Err(OsControlError::Unsupported {
                capability: CapabilityId::new("modify_scheduled_task"),
                reason: SafeText::new(
                    "the crontab backend has no stable per-entry identity or revision, so a compare-and-set patch cannot be expressed over it",
                ),
            });
        }
        selection::validate_timer_unit(task_id.as_str()).map(str::to_string)
    }

    /// The single facet of a patch this backend can apply and verify.
    ///
    /// A schedule or action patch is refused with a precise reason: rewriting a
    /// timer's calendar or its contained operation needs the governed
    /// unit-file writer, which is not composed, and editing the text by hand is
    /// exactly the ungoverned path this domain replaced.
    fn applicable_enablement(patch: &TypedAutomationPatch) -> Result<bool, OsControlError> {
        if patch.schedule.is_some() {
            return Err(OsControlError::Unsupported {
                capability: CapabilityId::new("modify_scheduled_task.schedule"),
                reason: SafeText::new(
                    "changing a timer's schedule requires the governed unit-file writer, which is not composed in this build",
                ),
            });
        }
        if patch.action.is_some() {
            return Err(OsControlError::Unsupported {
                capability: CapabilityId::new("modify_scheduled_task.action"),
                reason: SafeText::new(
                    "changing a task's contained action requires the governed unit-file writer, which is not composed in this build",
                ),
            });
        }
        patch.enabled.ok_or_else(|| {
            invalid("patch", "patch must change at least one applicable property")
        })
    }

    /// Observe the single fact a request's postcondition concerns.
    async fn observe_focus(
        &self,
        ctx: &HostExecutionContext,
        request: &AutomationRequest,
    ) -> Result<AutomationObservation, OsControlError> {
        match &request.op {
            AutomationOp::UpdateTask { task_id, .. } => {
                let unit = self.unit_of(task_id)?;
                let state = self.transport.read_task(ctx, &unit).await?;
                Ok(AutomationObservation::task_configuration(
                    task_id.as_str(),
                    state.present,
                    state.enabled,
                )
                .with_reported(state.revision, state.next_run_ms))
            }
            AutomationOp::RunWorkflow {
                workflow_id,
                expected_revision,
            } => {
                // Resolving the id first means an unknown workflow can never
                // reach a dispatch, and the revision compare happens against
                // the reviewed definition rather than a caller assertion.
                let descriptor = self.descriptor_for(workflow_id, *expected_revision)?;
                Ok(AutomationObservation::workflow_run(
                    workflow_id.as_str(),
                    0,
                    None,
                )
                .with_reported(Some(descriptor.revision), None))
            }
        }
    }

    /// Resolve a workflow id against the closed registry and check its
    /// revision.
    fn descriptor_for(
        &self,
        workflow_id: &WorkflowId,
        expected_revision: Revision,
    ) -> Result<&'static WorkflowDescriptor, OsControlError> {
        let descriptor = workflows::descriptor(self.registry, workflow_id).ok_or_else(|| {
            invalid(
                "workflow_id",
                "no reviewed workflow has this id; a workflow body can never be supplied by a caller",
            )
        })?;
        if descriptor.revision != expected_revision {
            return Err(OsControlError::TargetChanged);
        }
        if descriptor.max_step_risk() == RiskLevel::Black {
            return Err(invalid(
                "workflow_id",
                "a workflow containing a BLACK step may never run",
            ));
        }
        Ok(descriptor)
    }

    /// Build the governed structured command for a mutating operation.
    fn build_command(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        action: &str,
        params: &serde_json::Value,
        args: Vec<String>,
    ) -> Result<StructuredCommandRequest, OsControlError> {
        let executable = self.backend().trusted_executable()?;
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action.to_string(),
            params.clone(),
            executable,
            args,
        );
        StructuredCommandRequest::from_admitted(ctx, plan, &self.policy)
    }

    /// Compare the caller's `expected_revision` against a freshly read
    /// provider revision, immediately before mutating.
    fn check_revision(
        state: &AutomationTaskState,
        expected_revision: Revision,
    ) -> Result<(), OsControlError> {
        if !state.present {
            return Err(invalid(
                "task_id",
                "no scheduled task with this identity exists",
            ));
        }
        // A revision we could not read is not a revision that matches. Failing
        // closed here is what stops a patch being applied against a view the
        // caller never actually held.
        let Some(observed) = state.revision else {
            return Err(OsControlError::Unavailable {
                provider: None,
                reason: SafeText::new(
                    "the task's configuration revision could not be read; refusing a compare-and-set against an unknown revision",
                ),
                retryable: true,
            });
        };
        if observed != expected_revision {
            return Err(OsControlError::TargetChanged);
        }
        Ok(())
    }

    /// Execute a reviewed workflow's steps in order, verifying each one.
    ///
    /// A step that fails after an earlier step committed yields
    /// `PartiallyApplied` with the exact completed/failed step ids, so the
    /// receipt reports what really happened instead of a whole-run success or a
    /// whole-run failure.
    async fn run_steps(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &AutomationRequest,
        descriptor: &WorkflowDescriptor,
    ) -> Result<ApplyOutcome, OsControlError> {
        let mut completed: Vec<SafeStepId> = Vec::new();
        for step in descriptor.steps {
            let outcome = self.run_step(ctx, request, step).await;
            let step_failed = match outcome {
                Ok(ApplyOutcome::Applied(_)) => None,
                Ok(other) => Some(other),
                Err(error) => {
                    if completed.is_empty() {
                        // Nothing committed: the error is the whole story.
                        return Err(error);
                    }
                    Some(ApplyOutcome::Uncertain(UncertainDispatch::new(
                        None,
                        UncertainEffectCause::Unobservable,
                        BoundedVec::new(),
                    )))
                }
            };
            if step_failed.is_some() {
                let head = completed.first().cloned();
                return Ok(match head {
                    Some(head) => ApplyOutcome::PartiallyApplied(
                        crate::os_control::receipt::PartialDispatch::new(
                            None,
                            NonEmptyBoundedVec::new(
                                head,
                                BoundedVec::from_iter_capped(
                                    completed.into_iter().skip(1),
                                    workflows::WORKFLOW_MAX_STEPS,
                                ),
                            ),
                            SafeStepId::new(step.step_id),
                            PartialEffectCause::StepFailedAfterCommit,
                            BoundedVec::new(),
                        ),
                    ),
                    None => ApplyOutcome::Uncertain(UncertainDispatch::new(
                        None,
                        UncertainEffectCause::Unobservable,
                        BoundedVec::new(),
                    )),
                });
            }
            completed.push(SafeStepId::new(step.step_id));
        }
        Ok(ApplyOutcome::Applied(
            crate::os_control::receipt::AppliedDispatch::new(None, BoundedVec::new()),
        ))
    }

    /// Dispatch one typed workflow step and verify its own postcondition.
    async fn run_step(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &AutomationRequest,
        step: &WorkflowStep,
    ) -> Result<ApplyOutcome, OsControlError> {
        match step.action {
            WorkflowStepAction::SetTimerEnabled { unit, enabled } => {
                let unit = selection::validate_timer_unit(unit)?;
                let args = selection::set_enabled_argv(unit, enabled)?;
                let command =
                    self.build_command(ctx, &request.action, &request.params, args)?;
                let outcome = self.transport.dispatch(ctx, &command).await?;
                // Per-step verification: the step's own fact, read fresh.
                let state = self.transport.read_task(ctx.observation(), unit).await?;
                if state.enabled != Some(enabled) {
                    return Err(OsControlError::Unavailable {
                        provider: Some(self.provider_id()),
                        reason: SafeText::new(
                            "a workflow step did not reach its own postcondition",
                        ),
                        retryable: false,
                    });
                }
                Ok(outcome)
            }
        }
    }

    fn satisfying(
        &self,
        observed: &AutomationObservation,
    ) -> SatisfyingVerification<AutomationObservation> {
        SatisfyingVerification::new(
            self.evidence_source(),
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), observed.observation_digest()),
            None,
            SystemTime::now(),
            0,
        )
    }
}

#[async_trait]
impl<T: AutomationTransport> DesiredStateControl<AutomationRequest, AutomationObservation>
    for AutomationControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &AutomationRequest,
    ) -> Result<AutomationObservation, OsControlError> {
        self.observe_focus(ctx, request).await
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &AutomationRequest,
        _desired: &AutomationObservation,
    ) -> Result<ApplyOutcome, OsControlError> {
        match &request.op {
            AutomationOp::UpdateTask {
                task_id,
                expected_revision,
                patch,
            } => {
                let unit = self.unit_of(task_id)?;
                let enabled = Self::applicable_enablement(patch)?;

                // Fresh authoritative read, then the compare-and-set. Both
                // happen immediately before the change, never against a value
                // carried in from the caller's request.
                let state = self.transport.read_task(ctx.observation(), &unit).await?;
                Self::check_revision(&state, *expected_revision)?;

                if let Some(before) = state.enabled {
                    self.snapshots
                        .lock()
                        .expect("automation snapshots poisoned")
                        .insert(
                            ctx.grant().session_id().to_string(),
                            RollbackSnapshot {
                                before: AutomationObservation::task_configuration(
                                    task_id.as_str(),
                                    true,
                                    Some(before),
                                ),
                                action: request.action.clone(),
                                params: request.params.clone(),
                                unit: unit.clone(),
                            },
                        );
                } else {
                    return Err(OsControlError::Unsupported {
                        capability: CapabilityId::new("modify_scheduled_task.enabled"),
                        reason: SafeText::new(
                            "this unit has no enablement state (static or generated), so it cannot be enabled or disabled",
                        ),
                    });
                }

                let args = selection::set_enabled_argv(&unit, enabled)?;
                let command = self.build_command(ctx, &request.action, &request.params, args)?;
                self.transport.dispatch(ctx, &command).await
            }
            AutomationOp::RunWorkflow {
                workflow_id,
                expected_revision,
            } => {
                let descriptor = self.descriptor_for(workflow_id, *expected_revision)?;
                self.run_steps(ctx, request, descriptor).await
            }
        }
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &AutomationRequest,
        desired: &AutomationObservation,
    ) -> Result<VerificationReport<AutomationObservation>, OsControlError> {
        let observed = match &request.op {
            AutomationOp::UpdateTask { .. } => self.observe_focus(ctx, request).await?,
            AutomationOp::RunWorkflow {
                workflow_id,
                expected_revision,
            } => {
                // The aggregate postcondition: every reviewed step of the named
                // revision reached its own fact, each already verified during
                // the run.
                let descriptor = self.descriptor_for(workflow_id, *expected_revision)?;
                let mut completed = 0u32;
                let mut failed = None;
                for step in descriptor.steps {
                    match step.action {
                        WorkflowStepAction::SetTimerEnabled { unit, enabled } => {
                            let state = self.transport.read_task(ctx, unit).await?;
                            if state.enabled == Some(enabled) {
                                completed += 1;
                            } else if failed.is_none() {
                                failed = Some(step.step_id.to_string());
                            }
                        }
                    }
                }
                AutomationObservation::workflow_run(workflow_id.as_str(), completed, failed)
            }
        };

        // The desired workflow-run state is "every step completed, none
        // failed", filled in here from the reviewed definition rather than from
        // the caller's request.
        let desired = match &request.op {
            AutomationOp::RunWorkflow {
                workflow_id,
                expected_revision,
            } => {
                let descriptor = self.descriptor_for(workflow_id, *expected_revision)?;
                AutomationObservation::workflow_run(
                    workflow_id.as_str(),
                    u32::try_from(descriptor.steps.len()).unwrap_or(u32::MAX),
                    None,
                )
            }
            AutomationOp::UpdateTask { .. } => desired.clone(),
        };

        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(self.satisfying(&observed)))
        } else {
            Ok(VerificationReport::Contradicted(
                VerificationContradiction::new(
                    desired.observation_digest(),
                    Some(observed.observation_digest()),
                    SafeErrorCode::from_static("os_control.incident.contradicted"),
                ),
            ))
        }
    }

    async fn rollback(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        let snapshot = self
            .snapshots
            .lock()
            .expect("automation snapshots poisoned")
            .get(token.session_id().as_str())
            .cloned();

        // No recorded prior fact means there is no inverse we may claim.
        let Some(snapshot) = snapshot else {
            return Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
                None,
                UncertainEffectCause::Unobservable,
                BoundedVec::new(),
            )));
        };
        let Some(previous) = snapshot.before.enabled else {
            return Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
                None,
                UncertainEffectCause::Unobservable,
                BoundedVec::new(),
            )));
        };

        let args = selection::set_enabled_argv(&snapshot.unit, previous)?;
        let command = self.build_command(ctx, &snapshot.action, &snapshot.params, args)?;
        self.transport.dispatch(ctx, &command).await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping (existing tools/results stay compatible)
// ─────────────────────────────────────────────────────────────────────────────

/// Map an [`AutomationListing`] to the **existing** `list_scheduled_tasks`
/// result fields (`cron_jobs`, `systemd_timers`).
#[must_use]
pub fn list_scheduled_tasks_result(listing: &AutomationListing) -> serde_json::Value {
    serde_json::json!({
        "cron_jobs": listing.cron_jobs,
        "systemd_timers": listing.systemd_timers,
    })
}

/// Map a [`WorkflowPage`] to the frozen `WorkflowPage` result shape.
///
/// A workflow is identified by its stable `workflow_id`; no display name is
/// projected, because a name is neither unique nor stable.
#[must_use]
pub fn list_workflows_result(page: &WorkflowPage) -> serde_json::Value {
    serde_json::json!({
        "items": page
            .items
            .iter()
            .map(|w| serde_json::json!({
                "workflow_id": w.id,
                "revision": w.revision,
                "step_count": w.steps.len(),
                "risk": w.max_step_risk().as_str(),
            }))
            .collect::<Vec<_>>(),
        "next_cursor": page.next_cursor,
        "truncated": page.truncated,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::automation()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible automation domain port. Because the concrete
/// [`AutomationControl`] provider struct is generic over its
/// [`AutomationTransport`], `HostOsControl::automation()` returns this
/// object-safe supertrait instead so any transport (live, or a deny-live fake)
/// can be composed behind one erased reference.
#[async_trait]
pub trait AutomationControlPort:
    DesiredStateControl<AutomationRequest, AutomationObservation>
{
    /// Read the current cron/timer listing.
    async fn list(&self, ctx: &HostExecutionContext) -> Result<AutomationListing, OsControlError>;

    /// One page of reviewed workflows from the closed in-tree registry.
    fn list_workflows(
        &self,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<WorkflowPage, OsControlError>;

    /// Read one task's typed state, or the unambiguous absent fact.
    async fn read_task(
        &self,
        ctx: &HostExecutionContext,
        task_id: &AutomationId,
    ) -> Result<AutomationTaskState, OsControlError>;

    /// The selected backend.
    fn backend(&self) -> AutomationBackend;

    /// The composed provider identity.
    fn provider_id(&self) -> ProviderId;
}

#[async_trait]
impl<T: AutomationTransport> AutomationControlPort for AutomationControl<T> {
    async fn list(&self, ctx: &HostExecutionContext) -> Result<AutomationListing, OsControlError> {
        AutomationControl::list(self, ctx).await
    }

    fn list_workflows(
        &self,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<WorkflowPage, OsControlError> {
        AutomationControl::list_workflows(self, cursor, limit)
    }

    async fn read_task(
        &self,
        ctx: &HostExecutionContext,
        task_id: &AutomationId,
    ) -> Result<AutomationTaskState, OsControlError> {
        let unit = self.unit_of(task_id)?;
        self.transport.read_task(ctx, &unit).await
    }

    fn backend(&self) -> AutomationBackend {
        AutomationControl::backend(self)
    }

    fn provider_id(&self) -> ProviderId {
        AutomationControl::provider_id(self)
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn digest_binds_both_listing_fields() {
        let a = AutomationListing {
            cron_jobs: "0 * * * * foo".to_string(),
            systemd_timers: String::new(),
        };
        let b = a.clone();
        assert_eq!(a.observation_digest(), b.observation_digest());

        let c = AutomationListing {
            cron_jobs: "different".to_string(),
            systemd_timers: String::new(),
        };
        assert_ne!(a.observation_digest(), c.observation_digest());
    }

    #[test]
    fn focus_is_part_of_the_digest_so_domains_cannot_cross_verify() {
        let task = AutomationObservation::task_configuration("x.timer", true, Some(true));
        let run = AutomationObservation::workflow_run("x.timer", 0, None);
        assert_ne!(task.observation_digest(), run.observation_digest());
    }

    #[test]
    fn one_task_cannot_satisfy_another_tasks_postcondition() {
        let a = AutomationObservation::task_configuration("a.timer", true, Some(true));
        let b = AutomationObservation::task_configuration("b.timer", true, Some(true));
        assert_ne!(a.observation_digest(), b.observation_digest());
    }

    #[test]
    fn absent_disabled_and_unknown_are_three_different_facts() {
        let absent = AutomationObservation::task_configuration("a.timer", false, None);
        let disabled = AutomationObservation::task_configuration("a.timer", true, Some(false));
        let unknown = AutomationObservation::task_configuration("a.timer", true, None);
        assert_ne!(absent.observation_digest(), disabled.observation_digest());
        assert_ne!(absent.observation_digest(), unknown.observation_digest());
        assert_ne!(disabled.observation_digest(), unknown.observation_digest());
    }

    #[test]
    fn revision_is_a_precondition_not_part_of_the_postcondition_digest() {
        let before = AutomationObservation::task_configuration("a.timer", true, Some(true))
            .with_reported(Some(1), Some(10));
        let after = AutomationObservation::task_configuration("a.timer", true, Some(true))
            .with_reported(Some(2), Some(20));
        // The provider bumps the revision as a *result* of the change, so a
        // revision-bound digest would make every successful patch look
        // contradicted.
        assert_eq!(before.observation_digest(), after.observation_digest());
    }

    #[test]
    fn a_stale_revision_is_rejected_and_an_unreadable_one_fails_closed() {
        let fresh = AutomationTaskState {
            present: true,
            enabled: Some(true),
            revision: Some(7),
            next_run_ms: None,
        };
        assert!(matches!(
            AutomationControl::<fake::FakeAutomationTransport>::check_revision(&fresh, 6),
            Err(OsControlError::TargetChanged)
        ));
        assert!(AutomationControl::<fake::FakeAutomationTransport>::check_revision(&fresh, 7).is_ok());

        let unreadable = AutomationTaskState {
            present: true,
            enabled: Some(true),
            revision: None,
            next_run_ms: None,
        };
        assert!(matches!(
            AutomationControl::<fake::FakeAutomationTransport>::check_revision(&unreadable, 7),
            Err(OsControlError::Unavailable { .. })
        ));

        let absent = AutomationTaskState {
            present: false,
            enabled: None,
            revision: None,
            next_run_ms: None,
        };
        assert!(AutomationControl::<fake::FakeAutomationTransport>::check_revision(&absent, 7).is_err());
    }

    #[test]
    fn a_schedule_or_action_patch_is_refused_rather_than_approximated() {
        let schedule_patch = TypedAutomationPatch {
            schedule: Some(
                TypedSchedule::parse(&serde_json::json!({"kind":"once","run_at_ms":1}))
                    .expect("valid"),
            ),
            action: None,
            enabled: None,
        };
        assert!(matches!(
            AutomationControl::<fake::FakeAutomationTransport>::applicable_enablement(
                &schedule_patch
            ),
            Err(OsControlError::Unsupported { .. })
        ));

        let enabled_patch = TypedAutomationPatch {
            schedule: None,
            action: None,
            enabled: Some(false),
        };
        assert_eq!(
            AutomationControl::<fake::FakeAutomationTransport>::applicable_enablement(
                &enabled_patch
            )
            .expect("applicable"),
            false
        );
    }

    #[test]
    fn the_result_projection_uses_the_stable_id_only() {
        let page = WorkflowPage {
            items: Vec::new(),
            next_cursor: None,
            truncated: false,
        };
        let projected = list_workflows_result(&page);
        assert_eq!(projected["items"].as_array().map(Vec::len), Some(0));
        assert_eq!(projected["truncated"], serde_json::json!(false));
    }
}
