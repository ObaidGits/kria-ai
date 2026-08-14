//! Deny-live fake [`PackageTransport`] (OSC-014, OSC-033), Task 3.4.
//!
//! Compiled only under `os-control-test`. No PackageKit D-Bus call, no
//! `apt`/`dnf`/`pacman`/`zypper`/`snap`/`flatpak` subprocess and no
//! `pkexec`/`sudo` escalation is ever reached, so the process-wide deny-live
//! sentinel stays untripped.
//!
//! # Why this is a model, not a stub
//!
//! `apply_transaction` mutates the fake's own plan-standing table, so a
//! lifecycle test (observe → apply → re-observe → verify) exercises the real
//! governed path rather than a scripted sequence. Three deliberate
//! distinctions the real subsystem has, and a stub would flatten:
//!
//! * **"not applied" is not "could not determine".** An unscripted read
//!   errors and [`FakePlanStanding::Undetermined`] errors; only
//!   [`FakePlanStanding::NotApplied`] reports `applied: false`. Fabricating
//!   `false` for an unknown would let an install be skipped as
//!   already-satisfied, or a removal verify against a fact nobody read.
//! * **"in the database" is not "installed."**
//!   [`FakeInstallState::RemovedButNotPurged`] models Debian's
//!   removed-but-not-purged state: `is_installed()` is false (the removal
//!   *is* applied) while `is_present_in_database()` stays true (config files
//!   are retained, so a purge is still pending).
//! * **Applying a plan is not atomic.**
//!   [`FakePlanStanding::PartiallyApplied`] reports `applied: false` on every
//!   subsequent read, so a partial apply can never be verified as a full one.
//!
//! A plan is identified by its approved digest throughout
//! ([`PackagePlan::digest`]), so a state scripted for one plan can never
//! satisfy another's postcondition. A plan marked
//! [`FakePlanStanding::Stale`] — the world moved since approval — is refused
//! with [`OsControlError::TargetChanged`] unconditionally, including when an
//! apply outcome was scripted, so the refusal cannot be scripted away.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    BoundedVec, Digest, NonEmptyBoundedVec, ProviderId, SafeStepId, SafeText,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    AppliedDispatch, ApplyOutcome, PartialDispatch, PartialEffectCause,
};

use super::{
    PackageObservation, PackageOperation, PackagePage, PackagePlan, PackageProviderId, PackageRef,
    PackageTransactionState, PackageTransport, RebootRequirement, UpdateAssessment,
};

/// Provider identity reported by the fake transport. Matches the identity the
/// lifecycle harness binds its `MutationPlan` and `RollbackToken` to.
pub const FAKE_PACKAGE_PROVIDER_ID: &str = "packages-fake-packagekit";

/// Where an approved plan stands in the fake's world.
///
/// The four "not fully applied" cases are deliberately distinct facts, not
/// shades of one boolean: only [`Self::NotApplied`] is a clean
/// not-yet-applied plan that may be applied as approved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakePlanStanding {
    /// Not applied, and the plan still resolves exactly as approved.
    NotApplied,
    /// Every change in the plan is applied.
    FullyApplied,
    /// Some changes committed and at least one did not. Reads report
    /// `applied: false`: a partially applied plan is never a fully applied
    /// one, so it can never be verified as complete.
    PartiallyApplied,
    /// The world moved since approval — the plan no longer resolves to what
    /// was approved. Applying it is refused with
    /// [`OsControlError::TargetChanged`].
    Stale,
    /// The provider could not determine whether the plan is applied. A
    /// *different* fact from [`Self::NotApplied`]: reads error rather than
    /// reporting absence.
    Undetermined,
}

/// What the fake knows about one package's presence on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeInstallState {
    /// Installed at this version.
    Installed(String),
    /// The provider database was read and the package is genuinely absent —
    /// no entry, no residual configuration.
    NotInstalled,
    /// Present in the provider database but **not** installed: Debian's
    /// removed-but-not-purged state, holding the version whose configuration
    /// files were retained. The removal is applied; a purge is not.
    RemovedButNotPurged(String),
    /// The provider could not determine presence. Distinct from
    /// [`Self::NotInstalled`]: reads error instead of reporting absence.
    Undetermined,
}

impl FakeInstallState {
    /// Whether the package is installed and usable. False for both
    /// [`Self::NotInstalled`] and [`Self::RemovedButNotPurged`] — a
    /// removed-but-not-purged package is not installed.
    #[must_use]
    pub fn is_installed(&self) -> bool {
        matches!(self, Self::Installed(_))
    }

    /// Whether the provider database still holds an entry for the package.
    /// True for [`Self::RemovedButNotPurged`], which is what makes a pending
    /// *purge* distinguishable from a completed *removal*.
    #[must_use]
    pub fn is_present_in_database(&self) -> bool {
        matches!(self, Self::Installed(_) | Self::RemovedButNotPurged(_))
    }

    /// The installed version, when actually installed.
    #[must_use]
    pub fn installed_version(&self) -> Option<&str> {
        match self {
            Self::Installed(version) => Some(version),
            _ => None,
        }
    }

    /// The version whose configuration files were retained after a
    /// non-purging removal.
    #[must_use]
    pub fn residual_version(&self) -> Option<&str> {
        match self {
            Self::RemovedButNotPurged(version) => Some(version),
            _ => None,
        }
    }
}

/// A scripted, in-memory package transport.
///
/// Transaction-state reads are a FIFO queue because one governed mutation
/// performs several in a fixed order (pre-observation → under-lease
/// re-observation → post-apply re-observation → `verify`'s own fresh
/// re-read). Script them with successive [`Self::transaction_state_ok`]
/// calls. When the queue is exhausted the last scripted value is reused, so a
/// test that only cares about one steady state can script once.
///
/// With nothing queued, reads resolve against the plan-standing table (which
/// [`Self::apply_transaction`] mutates), then against the install-state
/// table, and finally fail closed. Nothing is ever invented.
pub struct FakePackageTransport {
    /// Explicit FIFO script for `read_transaction_state`; highest precedence.
    scripted_states: Mutex<VecDeque<Result<PackageTransactionState, OsControlError>>>,
    /// The last served scripted read, reused once the queue drains.
    last_state: Mutex<Option<Result<PackageTransactionState, OsControlError>>>,
    /// FIFO script for `apply_transaction`'s outcome.
    scripted_applies: Mutex<VecDeque<Result<ApplyOutcome, OsControlError>>>,
    /// The fake's world: how each approved plan digest stands.
    standings: Mutex<Vec<(Digest, FakePlanStanding)>>,
    /// Per-package presence, driving `get_info` / `list_installed` and the
    /// derived plan standing.
    install_states: Mutex<Vec<(PackageRef, FakeInstallState)>>,
    /// Scripted `get_package_info` observations, keyed by package.
    infos: Mutex<Vec<PackageObservation>>,
    /// Scripted `search_package` page. `None` means unscripted → fail closed.
    search: Mutex<Option<PackagePage>>,
    /// Scripted `list_installed_packages` page. `None` → derive from
    /// `install_states`.
    installed_page: Mutex<Option<PackagePage>>,
    /// Scripted `plan_package_changes` results, keyed by operation.
    plans: Mutex<Vec<(PackageOperation, PackagePlan)>>,
    /// Scripted `check_system_updates` assessment.
    updates: Mutex<Option<UpdateAssessment>>,
    /// Scripted `get_reboot_required` answer.
    reboot: Mutex<Option<RebootRequirement>>,
    /// Plan digests handed to `apply_transaction`, in order.
    dispatched: Mutex<Vec<Digest>>,
    /// Every transport operation reached, in order.
    labels: Mutex<Vec<String>>,
}

impl Default for FakePackageTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl FakePackageTransport {
    /// A fake with nothing scripted yet; every read fails closed until it is
    /// given a fact.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scripted_states: Mutex::new(VecDeque::new()),
            last_state: Mutex::new(None),
            scripted_applies: Mutex::new(VecDeque::new()),
            standings: Mutex::new(Vec::new()),
            install_states: Mutex::new(Vec::new()),
            infos: Mutex::new(Vec::new()),
            search: Mutex::new(None),
            installed_page: Mutex::new(None),
            plans: Mutex::new(Vec::new()),
            updates: Mutex::new(None),
            reboot: Mutex::new(None),
            dispatched: Mutex::new(Vec::new()),
            labels: Mutex::new(Vec::new()),
        }
    }

    /// Builder: queue the next `read_transaction_state` to return `state`.
    #[must_use]
    pub fn transaction_state_ok(self, state: PackageTransactionState) -> Self {
        self.scripted_states
            .lock()
            .expect("scripted states mutex")
            .push_back(Ok(state));
        self
    }

    /// Builder: queue the next `read_transaction_state` to fail, proving an
    /// ambiguous provider answer surfaces as an error rather than a
    /// fabricated `applied: false`.
    #[must_use]
    pub fn transaction_state_err(self, error: OsControlError) -> Self {
        self.scripted_states
            .lock()
            .expect("scripted states mutex")
            .push_back(Err(error));
        self
    }

    /// Builder: queue the next `apply_transaction` outcome. `Err(..)` models
    /// a denied or failed dispatch; the plan's standing is left untouched
    /// because no effect was proven.
    #[must_use]
    pub fn apply_outcome(self, outcome: Result<ApplyOutcome, OsControlError>) -> Self {
        self.scripted_applies
            .lock()
            .expect("scripted applies mutex")
            .push_back(outcome);
        self
    }

    /// Builder: record how an approved plan digest stands in the fake's
    /// world.
    #[must_use]
    pub fn plan_standing(self, digest: Digest, standing: FakePlanStanding) -> Self {
        self.set_standing(digest, standing);
        self
    }

    /// Builder: mark `plan` as applied (`true`) or cleanly not-yet-applied
    /// (`false`). Convenience over [`Self::plan_standing`].
    #[must_use]
    pub fn plan_applied(self, plan: &PackagePlan, applied: bool) -> Self {
        let standing = if applied {
            FakePlanStanding::FullyApplied
        } else {
            FakePlanStanding::NotApplied
        };
        self.plan_standing(plan.digest(), standing)
    }

    /// Builder: mark `plan` stale — the world changed since approval, so
    /// applying it must be refused rather than applied to a world that no
    /// longer matches the approval.
    #[must_use]
    pub fn stale_plan(self, plan: &PackagePlan) -> Self {
        self.plan_standing(plan.digest(), FakePlanStanding::Stale)
    }

    /// Builder: mark `plan` partially applied — some changes committed and at
    /// least one did not, so it must never read back as fully applied.
    #[must_use]
    pub fn partially_applied_plan(self, plan: &PackagePlan) -> Self {
        self.plan_standing(plan.digest(), FakePlanStanding::PartiallyApplied)
    }

    /// Builder: mark whether `plan` is applied as genuinely indeterminate.
    /// Reads error; they never report absence.
    #[must_use]
    pub fn undetermined_plan(self, plan: &PackagePlan) -> Self {
        self.plan_standing(plan.digest(), FakePlanStanding::Undetermined)
    }

    /// Builder: record one package's presence on the host.
    #[must_use]
    pub fn install_state(self, package: PackageRef, state: FakeInstallState) -> Self {
        let mut states = self.install_states.lock().expect("install states mutex");
        states.retain(|(existing, _)| existing != &package);
        states.push((package, state));
        drop(states);
        self
    }

    /// Builder: script one `get_package_info` observation.
    #[must_use]
    pub fn info_ok(self, observation: PackageObservation) -> Self {
        let mut infos = self.infos.lock().expect("infos mutex");
        infos.retain(|existing| existing.package != observation.package);
        infos.push(observation);
        drop(infos);
        self
    }

    /// Builder: script the `search_package` page (an empty page is a valid
    /// answer, distinct from an unscripted search).
    #[must_use]
    pub fn search_ok(self, page: PackagePage) -> Self {
        *self.search.lock().expect("search mutex") = Some(page);
        self
    }

    /// Builder: script the `list_installed_packages` page verbatim, bypassing
    /// derivation from the install-state table.
    #[must_use]
    pub fn installed_ok(self, page: PackagePage) -> Self {
        *self.installed_page.lock().expect("installed page mutex") = Some(page);
        self
    }

    /// Builder: script the plan `plan_package_changes` returns for
    /// `operation`.
    #[must_use]
    pub fn plan_ok(self, operation: PackageOperation, plan: PackagePlan) -> Self {
        let mut plans = self.plans.lock().expect("plans mutex");
        plans.retain(|(existing, _)| *existing != operation);
        plans.push((operation, plan));
        drop(plans);
        self
    }

    /// Builder: script the `check_system_updates` assessment.
    #[must_use]
    pub fn update_assessment_ok(self, assessment: UpdateAssessment) -> Self {
        *self.updates.lock().expect("updates mutex") = Some(assessment);
        self
    }

    /// Builder: script the `get_reboot_required` answer.
    #[must_use]
    pub fn reboot_required_ok(self, requirement: RebootRequirement) -> Self {
        *self.reboot.lock().expect("reboot mutex") = Some(requirement);
        self
    }

    /// How many times `apply_transaction` was reached. Counts attempts, not
    /// committed effects: a denied or refused apply still counts, which is
    /// what proves "dispatched exactly once".
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        self.dispatched.lock().expect("dispatched mutex").len()
    }

    /// The plan digests handed to `apply_transaction`, in order.
    #[must_use]
    pub fn dispatched_digests(&self) -> Vec<Digest> {
        self.dispatched.lock().expect("dispatched mutex").clone()
    }

    /// Every transport operation reached, in order.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        self.labels.lock().expect("labels mutex").clone()
    }

    /// The current standing of `plan` in the fake's world, if it has one.
    #[must_use]
    pub fn standing_of(&self, plan: &PackagePlan) -> Option<FakePlanStanding> {
        let digest = plan.digest();
        self.standings
            .lock()
            .expect("standings mutex")
            .iter()
            .find(|(known, _)| known == &digest)
            .map(|(_, standing)| *standing)
    }

    fn set_standing(&self, digest: Digest, standing: FakePlanStanding) {
        let mut standings = self.standings.lock().expect("standings mutex");
        standings.retain(|(known, _)| known != &digest);
        standings.push((digest, standing));
    }

    fn record(&self, label: &str) {
        self.labels
            .lock()
            .expect("labels mutex")
            .push(label.to_string());
    }

    /// The error an unscripted read returns. Never a value: a fake that
    /// invented state would let a test prove a mutation verified against a
    /// fact nobody read.
    fn unscripted(&self, reason: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_PACKAGE_PROVIDER_ID)),
            reason: SafeText::new(reason),
            retryable: false,
        }
    }

    fn indeterminate(&self, reason: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_PACKAGE_PROVIDER_ID)),
            reason: SafeText::new(reason),
            retryable: true,
        }
    }

    fn install_state_of(&self, package: &PackageRef) -> Option<FakeInstallState> {
        self.install_states
            .lock()
            .expect("install states mutex")
            .iter()
            .find(|(known, _)| known == package)
            .map(|(_, state)| state.clone())
    }

    /// Resolve a plan's standing from the per-package install-state table.
    ///
    /// An `Install`/`Update` plan is applied once every target is installed
    /// (at `to_version`, when the plan names one); a `Remove` plan is applied
    /// once no target is installed — which is true of a removed-but-not-
    /// purged package, whose removal genuinely did happen.
    ///
    /// Any target that is [`FakeInstallState::Undetermined`], or absent from
    /// the table entirely, yields `None`: the standing cannot be determined,
    /// and the caller must error rather than assume.
    fn derive_standing(&self, plan: &PackagePlan) -> Option<FakePlanStanding> {
        let targets: Vec<(&PackageRef, Option<&str>)> = match plan.operation {
            PackageOperation::Install | PackageOperation::Update => plan
                .installs
                .iter()
                .chain(plan.upgrades.iter())
                .map(|change| (&change.package, change.to_version.as_deref()))
                .collect(),
            PackageOperation::Remove => plan
                .removals
                .iter()
                .map(|change| (&change.package, None))
                .collect(),
        };
        if targets.is_empty() {
            return None;
        }

        let mut all_satisfied = true;
        for (package, wanted_version) in targets {
            let state = self.install_state_of(package)?;
            if matches!(state, FakeInstallState::Undetermined) {
                return None;
            }
            let satisfied = match plan.operation {
                PackageOperation::Install | PackageOperation::Update => match wanted_version {
                    Some(wanted) => state.installed_version() == Some(wanted),
                    None => state.is_installed(),
                },
                PackageOperation::Remove => !state.is_installed(),
            };
            all_satisfied &= satisfied;
        }

        Some(if all_satisfied {
            FakePlanStanding::FullyApplied
        } else {
            FakePlanStanding::NotApplied
        })
    }

    fn state_for_standing(
        &self,
        plan: &PackagePlan,
        standing: FakePlanStanding,
    ) -> Result<PackageTransactionState, OsControlError> {
        let applied = match standing {
            FakePlanStanding::FullyApplied => true,
            // A partially applied plan is not an applied plan, and a stale
            // plan's approved changes were never committed.
            FakePlanStanding::NotApplied
            | FakePlanStanding::PartiallyApplied
            | FakePlanStanding::Stale => false,
            FakePlanStanding::Undetermined => {
                return Err(self.indeterminate(
                    "provider could not determine whether the approved plan is applied",
                ))
            }
        };
        Ok(PackageTransactionState::new(
            plan.provider,
            plan.digest(),
            applied,
            plan.reboot_required,
        ))
    }
}

#[async_trait]
impl PackageTransport for FakePackageTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_PACKAGE_PROVIDER_ID)
    }

    async fn search(
        &self,
        _ctx: &HostExecutionContext,
        _query: &str,
        provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError> {
        self.record("search");
        let page = self
            .search
            .lock()
            .expect("search mutex")
            .clone()
            .ok_or_else(|| self.unscripted("no search page scripted on the fake transport"))?;
        Ok(paginate(page, provider, cursor, limit))
    }

    async fn get_info(
        &self,
        _ctx: &HostExecutionContext,
        package: &PackageRef,
    ) -> Result<PackageObservation, OsControlError> {
        self.record("get_info");
        self.resolve_info(package)
    }

    async fn list_installed(
        &self,
        _ctx: &HostExecutionContext,
        provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError> {
        self.record("list_installed");
        self.resolve_installed(provider, cursor, limit)
    }

    async fn plan(
        &self,
        _ctx: &HostExecutionContext,
        operation: PackageOperation,
        _packages: &[PackageRef],
    ) -> Result<PackagePlan, OsControlError> {
        self.record("plan");
        self.plans
            .lock()
            .expect("plans mutex")
            .iter()
            .find(|(known, _)| *known == operation)
            .map(|(_, plan)| plan.clone())
            .ok_or_else(|| self.unscripted("no plan scripted on the fake transport"))
    }

    async fn read_transaction_state(
        &self,
        _ctx: &HostExecutionContext,
        plan: &PackagePlan,
    ) -> Result<PackageTransactionState, OsControlError> {
        self.record("read_transaction_state");
        self.resolve_transaction_state(plan)
    }

    async fn apply_transaction(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        plan: &PackagePlan,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.record("apply_transaction");
        self.dispatched
            .lock()
            .expect("dispatched mutex")
            .push(plan.digest());
        self.resolve_apply(plan)
    }

    async fn assess_updates(
        &self,
        _ctx: &HostExecutionContext,
        _provider: Option<PackageProviderId>,
    ) -> Result<UpdateAssessment, OsControlError> {
        self.record("assess_updates");
        self.updates
            .lock()
            .expect("updates mutex")
            .clone()
            .ok_or_else(|| self.unscripted("no update assessment scripted on the fake transport"))
    }

    async fn reboot_required(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<RebootRequirement, OsControlError> {
        self.record("reboot_required");
        self.reboot
            .lock()
            .expect("reboot mutex")
            .clone()
            .ok_or_else(|| self.unscripted("no reboot requirement scripted on the fake transport"))
    }
}

/// The pure world-model half of the transport, split out of the trait methods
/// so every domain fact is unit-testable without assembling a full governed
/// mutation context.
impl FakePackageTransport {
    fn resolve_info(&self, package: &PackageRef) -> Result<PackageObservation, OsControlError> {
        if let Some(observation) = self
            .infos
            .lock()
            .expect("infos mutex")
            .iter()
            .find(|known| &known.package == package)
        {
            return Ok(observation.clone());
        }
        // Fall back to the presence table. `Undetermined` errors; it never
        // reports absence.
        match self.install_state_of(package) {
            Some(FakeInstallState::Undetermined) | None => Err(self.unscripted(
                "no package observation scripted on the fake transport for this package",
            )),
            Some(state) => Ok(PackageObservation {
                package: package.clone(),
                provider: package.provider(),
                installed_version: state.installed_version().map(str::to_string),
                candidate_version: None,
                origin: None,
                size_bytes: None,
                dependency_count: None,
                // Never fabricated: the fake reports no reboot metadata
                // unless a test scripts a full observation.
                reboot_implication: None,
            }),
        }
    }

    fn resolve_installed(
        &self,
        provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError> {
        if let Some(page) = self
            .installed_page
            .lock()
            .expect("installed page mutex")
            .clone()
        {
            return Ok(paginate(page, provider, cursor, limit));
        }
        let states = self.install_states.lock().expect("install states mutex");
        if states.is_empty() {
            return Err(self.unscripted("no installed packages scripted on the fake transport"));
        }
        // An indeterminate entry makes the whole listing a lie if omitted, so
        // it fails the read instead of quietly dropping the package.
        if states
            .iter()
            .any(|(_, state)| matches!(state, FakeInstallState::Undetermined))
        {
            return Err(self
                .indeterminate("provider could not determine presence for at least one package"));
        }
        // Only genuinely installed packages are listed: a removed-but-not-
        // purged entry exists in the database but is not installed.
        let items = states
            .iter()
            .filter(|(_, state)| state.is_installed())
            .map(|(package, state)| super::PackageEntry {
                package: package.clone(),
                provider: package.provider(),
                installed_version: state.installed_version().map(str::to_string),
                candidate_version: None,
                origin: None,
                size_bytes: None,
            })
            .collect();
        drop(states);
        Ok(paginate(
            PackagePage {
                items,
                truncated: false,
            },
            provider,
            cursor,
            limit,
        ))
    }

    fn resolve_transaction_state(
        &self,
        plan: &PackagePlan,
    ) -> Result<PackageTransactionState, OsControlError> {
        // 1) An explicit FIFO script wins; the last entry is held once the
        //    queue drains, so a steady state can be scripted once.
        let next = self
            .scripted_states
            .lock()
            .expect("scripted states mutex")
            .pop_front();
        let mut last = self.last_state.lock().expect("last state mutex");
        if let Some(value) = next {
            *last = Some(value);
        }
        if let Some(value) = last.clone() {
            return value;
        }
        drop(last);

        // 2) The fake's own world, which `apply_transaction` mutates.
        if let Some(standing) = self.standing_of(plan) {
            return self.state_for_standing(plan, standing);
        }

        // 3) Derived from the per-package presence table.
        if let Some(standing) = self.derive_standing(plan) {
            return self.state_for_standing(plan, standing);
        }

        // 4) Fail closed. Never `applied: false` for an unknown.
        Err(self
            .unscripted("no transaction state scripted on the fake transport for this plan digest"))
    }

    fn resolve_apply(&self, plan: &PackagePlan) -> Result<ApplyOutcome, OsControlError> {
        let digest = plan.digest();

        // A stale plan is refused unconditionally — before any scripted
        // outcome can claim otherwise. Applying a plan the world has moved
        // past would commit changes nobody approved.
        if self.standing_of(plan) == Some(FakePlanStanding::Stale) {
            return Err(OsControlError::TargetChanged);
        }

        if let Some(outcome) = self
            .scripted_applies
            .lock()
            .expect("scripted applies mutex")
            .pop_front()
        {
            match &outcome {
                Ok(ApplyOutcome::Applied(_)) | Ok(ApplyOutcome::Accepted(_)) => {
                    self.set_standing(digest, FakePlanStanding::FullyApplied);
                }
                Ok(ApplyOutcome::PartiallyApplied(_)) => {
                    self.set_standing(digest, FakePlanStanding::PartiallyApplied);
                }
                // Uncertain leaves the world genuinely unknown, and an error
                // proves no effect: neither may claim the plan is applied.
                Ok(ApplyOutcome::Uncertain(_)) => {
                    self.set_standing(digest, FakePlanStanding::Undetermined);
                }
                Err(_) => {}
            }
            return outcome;
        }

        match self.standing_of(plan) {
            Some(FakePlanStanding::PartiallyApplied) => {
                Ok(ApplyOutcome::PartiallyApplied(PartialDispatch::new(
                    None,
                    NonEmptyBoundedVec::single(SafeStepId::new("package-1")),
                    SafeStepId::new("package-2"),
                    PartialEffectCause::StepFailedAfterCommit,
                    BoundedVec::new(),
                )))
            }
            Some(FakePlanStanding::Undetermined) => Err(self
                .indeterminate("provider could not determine whether the approved plan applied")),
            // Unscripted or cleanly not-applied: apply the effect to the
            // fake's own world so a lifecycle test re-observes the change.
            _ => {
                self.set_standing(digest, FakePlanStanding::FullyApplied);
                Ok(ApplyOutcome::Applied(AppliedDispatch::new(
                    Some(Digest::of_str(crate::os_control::testing::FAKE_RECEIPT_TAG)),
                    BoundedVec::new(),
                )))
            }
        }
    }
}

/// Apply the provider filter and the cursor/limit window to a scripted page,
/// setting `truncated` honestly when entries remain beyond the window.
fn paginate(
    page: PackagePage,
    provider: Option<PackageProviderId>,
    cursor: usize,
    limit: usize,
) -> PackagePage {
    let filtered: Vec<super::PackageEntry> = page
        .items
        .into_iter()
        .filter(|item| provider.is_none_or(|wanted| item.provider == wanted))
        .collect();
    let total = filtered.len();
    let items: Vec<super::PackageEntry> = filtered.into_iter().skip(cursor).take(limit).collect();
    let truncated = page.truncated || cursor.saturating_add(items.len()) < total;
    PackagePage { items, truncated }
}

#[cfg(test)]
mod tests {
    use super::super::PackageChange;
    use super::*;

    fn htop() -> PackageRef {
        PackageRef::new(PackageProviderId::Apt, "htop")
    }

    fn install_plan() -> PackagePlan {
        PackagePlan {
            operation: PackageOperation::Install,
            provider: PackageProviderId::Apt,
            requested: vec![htop()],
            installs: vec![PackageChange {
                package: htop(),
                from_version: None,
                to_version: Some("3.0.5".to_string()),
            }],
            upgrades: vec![],
            removals: vec![],
            download_bytes: None,
            disk_delta_bytes: None,
            security_relevant: None,
            reboot_required: None,
        }
    }

    fn remove_plan() -> PackagePlan {
        PackagePlan {
            operation: PackageOperation::Remove,
            provider: PackageProviderId::Apt,
            requested: vec![htop()],
            installs: vec![],
            upgrades: vec![],
            removals: vec![PackageChange {
                package: htop(),
                from_version: Some("3.0.5".to_string()),
                to_version: None,
            }],
            download_bytes: None,
            disk_delta_bytes: None,
            security_relevant: None,
            reboot_required: None,
        }
    }

    #[test]
    fn an_unscripted_transaction_read_errors_rather_than_reporting_not_applied() {
        let fake = FakePackageTransport::new();
        let err = fake
            .resolve_transaction_state(&install_plan())
            .expect_err("an unknown plan state must error, never default to not-applied");
        assert!(matches!(err, OsControlError::Unavailable { .. }));
    }

    #[test]
    fn undetermined_is_a_different_fact_from_not_applied() {
        let plan = install_plan();

        let not_applied = FakePackageTransport::new().plan_applied(&plan, false);
        let state = not_applied
            .resolve_transaction_state(&plan)
            .expect("a known not-applied plan reads cleanly");
        assert!(!state.applied);

        let undetermined = FakePackageTransport::new().undetermined_plan(&plan);
        let err = undetermined
            .resolve_transaction_state(&plan)
            .expect_err("an indeterminate plan state must error, not read as not-applied");
        assert!(matches!(
            err,
            OsControlError::Unavailable {
                retryable: true,
                ..
            }
        ));
    }

    #[test]
    fn removed_but_not_purged_is_not_installed_so_a_removal_reads_as_applied() {
        let plan = remove_plan();
        let fake = FakePackageTransport::new().install_state(
            htop(),
            FakeInstallState::RemovedButNotPurged("3.0.5".to_string()),
        );

        let state = fake
            .resolve_transaction_state(&plan)
            .expect("presence table resolves the removal's standing");
        assert!(
            state.applied,
            "a removed-but-not-purged package is not installed, so the removal is applied"
        );

        let residual = fake.install_state_of(&htop()).expect("state recorded");
        assert!(!residual.is_installed());
        assert!(
            residual.is_present_in_database(),
            "config files are retained, so a purge is still pending"
        );
        assert_eq!(residual.residual_version(), Some("3.0.5"));

        // It is never listed as installed.
        let page = fake
            .resolve_installed(None, 0, 32)
            .expect("listing succeeds");
        assert!(page.items.is_empty());
    }

    #[test]
    fn an_installed_package_is_listed_and_reports_its_version() {
        let fake = FakePackageTransport::new()
            .install_state(htop(), FakeInstallState::Installed("3.0.5".to_string()));
        let page = fake
            .resolve_installed(None, 0, 32)
            .expect("listing succeeds");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].installed_version.as_deref(), Some("3.0.5"));

        let observation = fake.resolve_info(&htop()).expect("info resolves");
        assert_eq!(observation.installed_version.as_deref(), Some("3.0.5"));
        assert!(
            observation.reboot_implication.is_none(),
            "reboot implication is never fabricated"
        );
    }

    #[test]
    fn an_undetermined_presence_errors_instead_of_reporting_absence() {
        let fake =
            FakePackageTransport::new().install_state(htop(), FakeInstallState::Undetermined);
        assert!(fake.resolve_info(&htop()).is_err());
        assert!(fake.resolve_installed(None, 0, 32).is_err());
    }

    #[test]
    fn a_stale_plan_is_refused_and_cannot_be_scripted_into_applying() {
        let plan = install_plan();
        let fake = FakePackageTransport::new()
            .stale_plan(&plan)
            .apply_outcome(Ok(ApplyOutcome::Applied(AppliedDispatch::new(
                None,
                BoundedVec::new(),
            ))));

        let err = fake
            .resolve_apply(&plan)
            .expect_err("a stale plan must be refused");
        assert!(matches!(err, OsControlError::TargetChanged));
        assert_eq!(
            fake.standing_of(&plan),
            Some(FakePlanStanding::Stale),
            "a refused apply never promotes the plan to applied"
        );
        assert!(
            !fake
                .resolve_transaction_state(&plan)
                .expect("stale state reads")
                .applied,
            "a stale plan's approved changes were never committed"
        );
    }

    #[test]
    fn a_partially_applied_plan_never_reads_back_as_fully_applied() {
        let plan = install_plan();
        let fake = FakePackageTransport::new().partially_applied_plan(&plan);

        let outcome = fake
            .resolve_apply(&plan)
            .expect("a partial apply is an outcome, not an error");
        assert!(matches!(outcome, ApplyOutcome::PartiallyApplied(_)));

        let state = fake.resolve_transaction_state(&plan).expect("state reads");
        assert!(
            !state.applied,
            "a partially applied plan must never be reported as fully applied"
        );
    }

    #[test]
    fn apply_mutates_the_fakes_own_world_so_a_re_observation_sees_the_change() {
        let plan = install_plan();
        let fake = FakePackageTransport::new().plan_applied(&plan, false);

        let before = fake
            .resolve_transaction_state(&plan)
            .expect("pre-observation");
        assert!(!before.applied);

        fake.resolve_apply(&plan).expect("apply");

        let after = fake
            .resolve_transaction_state(&plan)
            .expect("re-observation");
        assert!(after.applied, "dispatch must apply the effect to the model");
        assert_eq!(after.plan_digest, plan.digest());
    }

    #[test]
    fn a_state_scripted_for_one_plan_never_satisfies_another_plans_digest() {
        let installing = install_plan();
        let removing = remove_plan();
        assert_ne!(installing.digest(), removing.digest());

        let fake = FakePackageTransport::new().plan_applied(&installing, true);
        let err = fake
            .resolve_transaction_state(&removing)
            .expect_err("a different plan digest has no scripted standing");
        assert!(matches!(err, OsControlError::Unavailable { .. }));
    }
}
