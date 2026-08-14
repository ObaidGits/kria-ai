//! Power domain: the `PowerControl` desired-state provider (design §3, §9.7)
//! — profile slice only.
//!
//! linux-os-control-production **Task 2.3** — "Migrate Wi-Fi and power-profile
//! controls" (OSC-020, OSC-029, OSC-031).
//!
//! This module replaces the direct `powerprofilesctl` subprocess handling that
//! used to live in `tools/system_config.rs` for `get_power_plan`/
//! `set_power_plan`. It composes the F1 runtime, mirroring
//! `os_control::audio`/`os_control::display`'s shape:
//!
//! * [`PowerProfileState`] is a normalized observation
//!   ([`NormalizedObservation`]) whose digest binds the exact profile name.
//! * [`PowerControl`] implements the generic [`DesiredStateControl`] lifecycle
//!   (observe → apply → verify → rollback) for `set_power_plan`. Its
//!   `apply`/`rollback` build a governed [`StructuredCommandRequest`] from the
//!   borrowed [`AdmittedMutationContext`] — the only sanctioned path to a
//!   child process — so no power code touches `ExecWrapper`/`tokio::process`
//!   directly.
//! * The live transport
//!   ([`crate::os_control::linux::providers::power_profiles`]) is a raw,
//!   deny-live-gated adapter; deny-live tests inject
//!   [`FakePowerProfileTransport`].
//!
//! # Scope boundary
//!
//! This module implements the profile read/set slice of `PowerControl`
//! (design §9.7's `get_profile`/`set_profile`). The session/lifecycle slice —
//! lock/suspend/hibernate/shutdown/reboot — is implemented as a sibling port
//! in [`session`] (Task 2.4). Logout and battery-threshold operations remain a
//! distinct later task (3.8) and are not represented here.

/// Battery charge thresholds, applied through the privileged broker.
pub mod charge;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest,
};
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

pub mod parsers;
pub mod selection;

/// Deny-live fake transport (Task 0.4 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;


// ── Task 2.4: power session/lifecycle slice (lock/suspend/hibernate/
// shutdown/reboot) ───────────────────────────────────────────────────────────
// A sibling slice within the same `power` domain module (see `session::mod`
// docs for why it is a distinct `DesiredStateControl` port rather than an
// extension of the profile-slice `PowerControl` above).
pub mod session;

pub use selection::PowerProfileBackend;

/// The closed set of power profiles the frozen manifest's `set_power_plan`
/// schema declares (`power_saver` / `balanced` / `performance`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerProfile {
    /// Reduced performance for extended battery life.
    PowerSaver,
    /// The default balanced profile.
    Balanced,
    /// Maximum performance, at the cost of battery life/thermals.
    Performance,
}

impl PowerProfile {
    /// The stable token used on the wire and by `powerprofilesctl`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PowerProfile::PowerSaver => "power-saver",
            PowerProfile::Balanced => "balanced",
            PowerProfile::Performance => "performance",
        }
    }

    /// Parse a profile token. Accepts both the manifest's `power_saver` and the
    /// `powerprofilesctl`-native `power-saver` spelling (they normalize to the
    /// same profile); unrecognized input is `None` — ambiguity never reports a
    /// fabricated profile.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_lowercase().replace('_', "-").as_str() {
            "power-saver" => Some(PowerProfile::PowerSaver),
            "balanced" => Some(PowerProfile::Balanced),
            "performance" => Some(PowerProfile::Performance),
            _ => None,
        }
    }
}

/// A normalized power-profile observation (design §5, §9.7).
#[derive(Debug, Clone, PartialEq)]
pub struct PowerProfileState {
    /// The currently active profile.
    pub profile: PowerProfile,
}

/// A battery-health observation (`get_battery_health`, Task 3.8, OSC-020).
///
/// This is a **read-only** domain fact, not a desired-state observation: the
/// frozen contract declares `verificationClass: None` and `rollbackClaim: None`
/// for `get_battery_health`, so it has no digest and never participates in
/// idempotency or verification.
///
/// The two variants exist because *absent* and *unknown* are different facts
/// (design §5): a desktop with no battery is [`BatteryHealth::Absent`], while a
/// battery whose capacity cannot be read is an `Err` from the transport. Neither
/// is ever reported as `0` percent health, which would describe a healthy or
/// missing battery as a dying one.
#[derive(Debug, Clone, PartialEq)]
pub enum BatteryHealth {
    /// No battery is present on this host — a positive fact read from the power
    /// service's own device inventory, not an inference from a failed read.
    Absent,
    /// A battery is present and its health was read.
    Present {
        /// Full-charge capacity as a percentage of design capacity (1..=100).
        capacity_percent: u8,
        /// Charge cycles, when the driver reports them. `None` means "not
        /// reported"; it is never collapsed into `0`.
        cycle_count: Option<u64>,
        /// The declared health band for `capacity_percent`
        /// (see [`selection::classify_battery_health`]).
        health_state: &'static str,
    },
}

impl BatteryHealth {
    /// Construct a present-battery observation, deriving the health band from
    /// the measured capacity so the band and the number can never disagree.
    #[must_use]
    pub fn present(capacity_percent: u8, cycle_count: Option<u64>) -> Self {
        Self::Present {
            capacity_percent,
            cycle_count,
            health_state: selection::classify_battery_health(capacity_percent),
        }
    }

    /// Whether a battery is present.
    #[must_use]
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present { .. })
    }
}

impl PowerProfileState {
    /// Construct a profile observation.
    #[must_use]
    pub fn new(profile: PowerProfile) -> Self {
        Self { profile }
    }
}

impl NormalizedObservation for PowerProfileState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!("power:profile:{}", self.profile.as_str()))
    }
}

/// The concrete power-profile operation this task migrates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerProfileOp {
    /// Read-only state query (`get_power_plan`).
    GetProfile,
    /// Set the active power profile (`set_power_plan`).
    SetProfile(PowerProfile),
}

/// A fully-described power-profile request. Carries the canonical `action`/
/// `params` so the governed [`StructuredCommandRequest`] can bind them against
/// the grant.
#[derive(Debug, Clone)]
pub struct PowerProfileRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The concrete operation.
    pub op: PowerProfileOp,
}

impl PowerProfileRequest {
    /// The desired end state for a mutation. Returns `None` for the read-only
    /// [`PowerProfileOp::GetProfile`].
    #[must_use]
    pub fn desired_state(&self) -> Option<PowerProfileState> {
        match self.op {
            PowerProfileOp::GetProfile => None,
            PowerProfileOp::SetProfile(p) => Some(PowerProfileState::new(p)),
        }
    }

    /// The idempotency/verification comparator (the frozen manifest names
    /// `ExactTypedPostcondition` for `set_power_plan`).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw power-profile transport seam. The live implementation
/// ([`crate::os_control::linux::providers::power_profiles::LivePowerProfiles`])
/// is a deny-live-gated adapter over `power-profiles-daemon` D-Bus (structured
/// `powerprofilesctl` fallback until wired); deny-live tests inject
/// [`FakePowerProfileTransport`]. Reads run a query/parse; `dispatch` runs a
/// governed [`StructuredCommandRequest`].
#[async_trait]
pub trait PowerProfileTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// The selected backend.
    fn selected_backend(&self) -> PowerProfileBackend;

    /// Read the current active profile. A parse ambiguity must surface as an
    /// error, never a fabricated state.
    async fn read_profile(&self, ctx: &HostExecutionContext) -> Result<PowerProfile, OsControlError>;

    /// Read battery health (`get_battery_health`, Task 3.8).
    ///
    /// [`BatteryHealth::Absent`] must be returned only when the power service's
    /// own device inventory says there is no battery. Anything the transport
    /// cannot read is an `Err`, so "no battery" and "could not tell" stay
    /// distinct facts.
    ///
    /// The default refuses rather than inventing a reading, so a transport that
    /// composes no battery source reports `Unavailable` instead of a fabricated
    /// `0` percent health.
    async fn read_battery_health(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<BatteryHealth, OsControlError> {
        Err(OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: crate::os_control::contract::SafeText::new(
                "this power transport composes no battery source; battery health is unknown, not zero",
            ),
            retryable: false,
        })
    }

    /// Dispatch a governed structured command (the only path to a process).
    async fn dispatch(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The rollback snapshot captured before an apply, so a contradiction can be
/// compensated back to the exact prior profile.
#[derive(Debug, Clone)]
struct RollbackSnapshot {
    before_profile: PowerProfile,
    action: String,
    params: serde_json::Value,
}

/// The `PowerControl` profile-slice provider (design §3, §4, §9.7). Generic
/// over the [`PowerProfileTransport`] so the same governed logic runs over the
/// live `power-profiles-daemon`/`powerprofilesctl` adapter and the deny-live
/// fake.
pub struct PowerControl<T: PowerProfileTransport> {
    transport: T,
    policy: CommandPolicy,
    /// Prior-profile snapshots keyed by session id, captured in `apply` for
    /// `rollback`. Interior mutability because the provider is shared (`&self`);
    /// power-profile ops are serialized by the `power-profile/system` resource
    /// lease.
    snapshots: Mutex<HashMap<String, RollbackSnapshot>>,
}

impl<T: PowerProfileTransport> PowerControl<T> {
    /// Compose a `PowerControl` over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            policy: CommandPolicy::new(),
            snapshots: Mutex::new(HashMap::new()),
        }
    }

    /// The selected backend (for the `backend` result field).
    #[must_use]
    pub fn backend(&self) -> PowerProfileBackend {
        self.transport.selected_backend()
    }

    /// Borrow the underlying transport (used by tests to inspect captured argv).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The provider identity.
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        self.transport.provider_id()
    }

    fn evidence_source(&self) -> OsEvidenceSource {
        match self.transport.selected_backend() {
            PowerProfileBackend::PowerProfilesDaemon => OsEvidenceSource::AuthoritativeServiceState,
            PowerProfileBackend::Powerprofilesctl => OsEvidenceSource::StructuredCommandQuery,
        }
    }

    /// Build the governed structured command for a mutating operation.
    fn build_command(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        action: &str,
        params: &serde_json::Value,
        args: Vec<String>,
    ) -> Result<StructuredCommandRequest, OsControlError> {
        let backend = self.transport.selected_backend();
        let executable = backend.trusted_executable()?;
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action.to_string(),
            params.clone(),
            executable,
            args,
        );
        StructuredCommandRequest::from_admitted(ctx, plan, &self.policy)
    }

    fn satisfying(&self, observed: &PowerProfileState) -> SatisfyingVerification<PowerProfileState> {
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
impl<T: PowerProfileTransport> DesiredStateControl<PowerProfileRequest, PowerProfileState>
    for PowerControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        _request: &PowerProfileRequest,
    ) -> Result<PowerProfileState, OsControlError> {
        let profile = self.transport.read_profile(ctx).await?;
        Ok(PowerProfileState::new(profile))
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &PowerProfileRequest,
        _desired: &PowerProfileState,
    ) -> Result<ApplyOutcome, OsControlError> {
        if let Ok(before_profile) = self.transport.read_profile(ctx.observation()).await {
            let session = ctx.grant().session_id().to_string();
            self.snapshots
                .lock()
                .expect("power snapshots poisoned")
                .insert(
                    session,
                    RollbackSnapshot {
                        before_profile,
                        action: request.action.clone(),
                        params: request.params.clone(),
                    },
                );
        }

        let PowerProfileOp::SetProfile(profile) = request.op else {
            return Err(OsControlError::InvalidRequest {
                field: crate::os_control::contract::SafeField::new("op"),
                reason: crate::os_control::contract::SafeText::new(
                    "get_power_plan has no mutation to apply",
                ),
            });
        };
        let args = selection::set_profile_argv(self.transport.selected_backend(), profile);
        let command = self.build_command(ctx, &request.action, &request.params, args)?;
        self.transport.dispatch(ctx, &command).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        _request: &PowerProfileRequest,
        desired: &PowerProfileState,
    ) -> Result<VerificationReport<PowerProfileState>, OsControlError> {
        let profile = self.transport.read_profile(ctx).await?;
        let observed = PowerProfileState::new(profile);

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
            .expect("power snapshots poisoned")
            .get(token.session_id().as_str())
            .cloned();

        let Some(snapshot) = snapshot else {
            return Ok(ApplyOutcome::Uncertain(
                crate::os_control::receipt::UncertainDispatch::new(
                    None,
                    crate::os_control::receipt::UncertainEffectCause::Unobservable,
                    crate::os_control::contract::BoundedVec::new(),
                ),
            ));
        };

        let args = selection::set_profile_argv(
            self.transport.selected_backend(),
            snapshot.before_profile,
        );
        let command =
            self.build_command(ctx, &snapshot.action, &snapshot.params, args)?;
        self.transport.dispatch(ctx, &command).await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping (existing tools/results stay compatible)
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a governed [`MutationReceipt`] to the **existing** `set_power_plan`
/// result fields (`power_plan`, `changed`, `already_in_desired_state`), plus
/// additive `backend`/`lifecycle`/`verified` fields (design §9.7, Task 2.3).
#[must_use]
pub fn set_power_plan_result(
    receipt: &MutationReceipt<PowerProfileState>,
    requested: PowerProfile,
    backend: PowerProfileBackend,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "power_plan": requested.as_str(),
        "backend": backend.as_str(),
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a read-only [`PowerProfileState`] to the `get_power_plan` result fields.
#[must_use]
pub fn power_plan_result(state: &PowerProfileState) -> serde_json::Value {
    serde_json::json!({ "power_plan": state.profile.as_str() })
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::power()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible power domain port design §4 names
/// `fn power(&self) -> &dyn PowerControl` on `HostOsControl`. Because the
/// concrete [`PowerControl`] provider struct above is generic over its
/// [`PowerProfileTransport`], `HostOsControl::power()` returns this
/// object-safe supertrait instead so any transport (live
/// `power-profiles-daemon`/`powerprofilesctl`, or a deny-live fake) can be
/// composed behind one erased reference. Every [`PowerControl<T>`] implements
/// it automatically via the blanket impl below.
#[async_trait]
pub trait PowerControlPort: DesiredStateControl<PowerProfileRequest, PowerProfileState> {
    /// Read battery health (`get_battery_health`).
    ///
    /// A read, so it takes only the admitted observation context — there is no
    /// grant to seal because nothing changes.
    async fn read_battery_health(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<BatteryHealth, OsControlError>;
}

#[async_trait]
impl<T: PowerProfileTransport> PowerControlPort for PowerControl<T> {
    async fn read_battery_health(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<BatteryHealth, OsControlError> {
        self.transport.read_battery_health(ctx).await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn profile_parse_accepts_both_underscore_and_hyphen_spellings() {
        assert_eq!(PowerProfile::parse("power_saver"), Some(PowerProfile::PowerSaver));
        assert_eq!(PowerProfile::parse("power-saver"), Some(PowerProfile::PowerSaver));
        assert_eq!(PowerProfile::parse("balanced"), Some(PowerProfile::Balanced));
        assert_eq!(PowerProfile::parse("performance"), Some(PowerProfile::Performance));
        assert_eq!(PowerProfile::parse("turbo"), None);
        assert_eq!(PowerProfile::parse(""), None);
    }

    #[test]
    fn observation_digest_binds_exact_profile() {
        let a = PowerProfileState::new(PowerProfile::Balanced);
        let b = PowerProfileState::new(PowerProfile::Balanced);
        assert_eq!(a.observation_digest(), b.observation_digest());
        let c = PowerProfileState::new(PowerProfile::Performance);
        assert_ne!(a.observation_digest(), c.observation_digest());
    }

    #[test]
    fn desired_state_none_for_read_only_get_profile() {
        let request = PowerProfileRequest {
            action: "get_power_plan".to_string(),
            params: serde_json::json!({}),
            op: PowerProfileOp::GetProfile,
        };
        assert!(request.desired_state().is_none());
    }
}
