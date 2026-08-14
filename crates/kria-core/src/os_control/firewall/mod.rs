//! Firewall status, enable/disable, and temporary per-application access.
//!
//! linux-os-control-production tasks **4.3** and **5.3** (OSC-017).
//!
//! # The asymmetry that shapes the risk levels
//!
//! `set_firewall_enabled` is **RED when disabling** and YELLOW when enabling. That
//! is not a formality: turning a firewall off exposes every listening service on
//! the machine at once, and it is the one direction a mistaken command cannot be
//! walked back from — packets that got in, got in. Turning it on can at worst
//! break a connection the user then notices.
//!
//! # Temporary access is a promise the domain must keep
//!
//! `grant_temporary_app_network_access` opens a hole for a bounded time. A grant
//! that silently outlived its duration would be worse than never expiring at all,
//! because the user would believe it had closed. So:
//!
//! * the duration is **bounded and required** — there is no "forever" option;
//! * the expiry is recorded with the rule, so a later read reports it honestly;
//! * the receipt claims **no rollback**, because revocation is the expiry itself,
//!   not a compensating action taken from here.
//!
//! # Fail closed, in the protective direction
//!
//! An unreadable firewall state is **unknown**, never "enabled". Reporting a
//! firewall as active when it could not be read would tell the user they are
//! protected while they may be wide open.

use async_trait::async_trait;

use crate::os_control::broker::protocol::FirewallProviderId;
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, SafeField, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

/// The provider identity.
pub const FIREWALL_PROVIDER_ID: &str = "firewall";

/// The shortest temporary grant that is meaningful.
pub const MIN_GRANT_MS: u64 = 1_000;

/// The longest temporary grant this domain will issue.
///
/// Bounded on purpose: "temporary" must mean temporary. A grant measured in days
/// is a permanent rule with extra steps, and the user would stop expecting it.
pub const MAX_GRANT_MS: u64 = 4 * 60 * 60 * 1_000;

/// Which fact an observation carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallFocus {
    /// Whether the firewall itself is enabled.
    Enabled,
    /// Whether a temporary application rule is present.
    AppGrant,
}

impl FirewallFocus {
    fn tag(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::AppGrant => "app-grant",
        }
    }
}

/// A normalized firewall observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirewallState {
    /// Which fact this carries.
    pub focus: FirewallFocus,
    /// The resolved provider.
    pub provider: FirewallProviderId,
    /// Whether the firewall is enabled, when that is the focus.
    pub enabled: Option<bool>,
    /// The application a grant is about, when that is the focus.
    pub app_id: Option<String>,
    /// Whether a temporary rule for that application is present.
    pub grant_present: bool,
}

impl NormalizedObservation for FirewallState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "firewall:{}:{}:{}:{}:{}",
            self.focus.tag(),
            self.provider.tag(),
            self.enabled.map_or_else(|| "-".to_string(), |v| v.to_string()),
            self.app_id.as_deref().unwrap_or("-"),
            self.grant_present,
        ))
    }
}

/// Facts read from the firewall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirewallFacts {
    /// The resolved provider.
    pub provider: FirewallProviderId,
    /// Whether the firewall is enabled. `None` is **unknown**, never "on".
    pub enabled: Option<bool>,
    /// Default policy token for incoming traffic, when reported.
    pub default_incoming: Option<String>,
    /// How many rules are configured, when reported.
    pub rule_count: Option<u32>,
}

/// A bounded temporary-access duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantDuration(u64);

impl GrantDuration {
    /// Validate a requested duration.
    pub fn parse(ms: u64) -> Result<Self, OsControlError> {
        if !(MIN_GRANT_MS..=MAX_GRANT_MS).contains(&ms) {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("duration"),
                reason: SafeText::new(
                    "duration must be between 1 second and 4 hours: a temporary network grant that \
                     lasts longer is a permanent rule the user would stop expecting",
                ),
            });
        }
        Ok(Self(ms))
    }

    /// The duration in milliseconds.
    #[must_use]
    pub fn as_millis(self) -> u64 {
        self.0
    }
}

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirewallOp {
    /// Enable or disable the firewall.
    SetEnabled(bool),
    /// Grant one application bounded network access.
    GrantTemporary {
        /// The application's stable id.
        app_id: String,
        /// How long the grant lasts.
        duration: GrantDuration,
    },
}

impl FirewallOp {
    /// The fact this operation is judged against.
    #[must_use]
    pub fn focus(&self) -> FirewallFocus {
        match self {
            Self::SetEnabled(_) => FirewallFocus::Enabled,
            Self::GrantTemporary { .. } => FirewallFocus::AppGrant,
        }
    }

    /// Whether this operation reduces protection.
    ///
    /// Surfaced so a caller/receipt can state plainly which direction was taken;
    /// the risk level itself is fixed by the frozen contract.
    #[must_use]
    pub fn reduces_protection(&self) -> bool {
        match self {
            Self::SetEnabled(enabled) => !*enabled,
            // A temporary grant opens a hole, bounded but real.
            Self::GrantTemporary { .. } => true,
        }
    }
}

/// One governed firewall request.
#[derive(Debug, Clone)]
pub struct FirewallRequest {
    /// The canonical tool/action name.
    pub action: String,
    /// The canonical tool parameters.
    pub params: serde_json::Value,
    /// The operation.
    pub op: FirewallOp,
}

impl FirewallRequest {
    /// The state this request is trying to reach.
    #[must_use]
    pub fn desired_state(&self, observed: &FirewallState) -> FirewallState {
        match &self.op {
            FirewallOp::SetEnabled(enabled) => FirewallState {
                focus: FirewallFocus::Enabled,
                provider: observed.provider,
                enabled: Some(*enabled),
                app_id: None,
                grant_present: observed.grant_present,
            },
            FirewallOp::GrantTemporary { app_id, .. } => FirewallState {
                focus: FirewallFocus::AppGrant,
                provider: observed.provider,
                enabled: observed.enabled,
                app_id: Some(app_id.clone()),
                // The postcondition is that the rule EXISTS. Its expiry is the
                // firewall's business, not something verified here.
                grant_present: true,
            },
        }
    }

    /// The comparator.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

/// The raw transport.
#[async_trait]
pub trait FirewallTransport: Send + Sync {
    /// The provider identity.
    fn provider_id(&self) -> ProviderId;

    /// Read firewall facts.
    async fn read_facts(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<FirewallFacts, OsControlError>;

    /// Whether a temporary rule for `app_id` is present.
    async fn read_app_grant(
        &self,
        ctx: &HostExecutionContext,
        app_id: &str,
    ) -> Result<bool, OsControlError>;

    /// Apply one operation.
    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        op: &FirewallOp,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The governed provider.
pub struct FirewallControl<T: FirewallTransport> {
    transport: T,
}

impl<T: FirewallTransport> FirewallControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the transport.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn satisfying(&self, observed: &FirewallState) -> SatisfyingVerification<FirewallState> {
        let digest = observed.observation_digest();
        SatisfyingVerification::new(
            OsEvidenceSource::AuthoritativeServiceState,
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), digest),
            None,
            std::time::SystemTime::now(),
            0,
        )
    }
}

#[async_trait]
impl<T: FirewallTransport> DesiredStateControl<FirewallRequest, FirewallState>
    for FirewallControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &FirewallRequest,
    ) -> Result<FirewallState, OsControlError> {
        let facts = self.transport.read_facts(ctx).await?;

        // An unreadable firewall state fails closed. Reporting "enabled" would tell
        // the user they are protected when they may be wide open.
        if matches!(request.op, FirewallOp::SetEnabled(_)) && facts.enabled.is_none() {
            return Err(OsControlError::Unavailable {
                provider: Some(self.transport.provider_id()),
                reason: SafeText::new(
                    "the firewall's state could not be read; it is unknown, not enabled",
                ),
                retryable: true,
            });
        }

        let (app_id, grant_present) = match &request.op {
            FirewallOp::GrantTemporary { app_id, .. } => (
                Some(app_id.clone()),
                self.transport.read_app_grant(ctx, app_id).await?,
            ),
            FirewallOp::SetEnabled(_) => (None, false),
        };

        Ok(FirewallState {
            focus: request.op.focus(),
            provider: facts.provider,
            enabled: facts.enabled,
            app_id,
            grant_present,
        })
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &FirewallRequest,
        _desired: &FirewallState,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.transport.apply(ctx, &request.op).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &FirewallRequest,
        desired: &FirewallState,
    ) -> Result<VerificationReport<FirewallState>, OsControlError> {
        let observed = self.observe(ctx, request).await?;
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
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // A temporary grant is revoked by its own expiry, and re-enabling a
        // firewall is a user-requested action per the contract. Neither is an
        // automatic compensation from here.
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("firewall.rollback"),
            reason: SafeText::new(
                "a temporary grant ends at its expiry, and the firewall is re-enabled by an \
                 explicit user request",
            ),
        })
    }
}

/// The port a handler resolves.
#[async_trait]
pub trait FirewallControlPort: DesiredStateControl<FirewallRequest, FirewallState> {
    /// Read the firewall's status.
    async fn status(&self, ctx: &HostExecutionContext) -> Result<FirewallFacts, OsControlError>;
}

#[async_trait]
impl<T: FirewallTransport> FirewallControlPort for FirewallControl<T> {
    async fn status(&self, ctx: &HostExecutionContext) -> Result<FirewallFacts, OsControlError> {
        self.transport.read_facts(ctx).await
    }
}

/// Validate an application id before it becomes part of a firewall rule.
pub fn validate_app_id(raw: &str) -> Result<String, OsControlError> {
    let raw = raw.trim();
    let ok = !raw.is_empty()
        && raw.len() <= 128
        && !raw.starts_with('-')
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !ok {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new("app_id"),
            reason: SafeText::new(
                "app_id must be a stable application id; a value starting with `-` would be read \
                 as a command option",
            ),
        });
    }
    Ok(raw.to_string())
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn state(focus: FirewallFocus, enabled: Option<bool>, grant: bool) -> FirewallState {
        FirewallState {
            focus,
            provider: FirewallProviderId::Ufw,
            enabled,
            app_id: Some("org.example.App".to_string()),
            grant_present: grant,
        }
    }

    #[test]
    fn disabling_is_flagged_as_reducing_protection() {
        assert!(FirewallOp::SetEnabled(false).reduces_protection());
        assert!(!FirewallOp::SetEnabled(true).reduces_protection());
        // A bounded hole is still a hole.
        assert!(FirewallOp::GrantTemporary {
            app_id: "a".to_string(),
            duration: GrantDuration::parse(5_000).unwrap(),
        }
        .reduces_protection());
    }

    #[test]
    fn a_grant_duration_outside_the_bound_is_refused() {
        assert!(GrantDuration::parse(0).is_err());
        assert!(GrantDuration::parse(500).is_err(), "under one second");
        // A day-long "temporary" grant is a permanent rule with extra steps.
        assert!(GrantDuration::parse(24 * 60 * 60 * 1_000).is_err());
        assert_eq!(GrantDuration::parse(60_000).unwrap().as_millis(), 60_000);
        assert!(GrantDuration::parse(MAX_GRANT_MS).is_ok());
    }

    #[test]
    fn unknown_enabled_state_is_distinct_from_enabled() {
        // The protective distinction: "could not read" must not equal "protected".
        assert_ne!(
            state(FirewallFocus::Enabled, None, false).observation_digest(),
            state(FirewallFocus::Enabled, Some(true), false).observation_digest(),
        );
    }

    #[test]
    fn focus_is_part_of_the_digest() {
        assert_ne!(
            state(FirewallFocus::Enabled, Some(true), true).observation_digest(),
            state(FirewallFocus::AppGrant, Some(true), true).observation_digest(),
        );
    }

    #[test]
    fn a_grant_desires_the_rule_to_exist() {
        let observed = state(FirewallFocus::AppGrant, Some(true), false);
        let request = FirewallRequest {
            action: "grant_temporary_app_network_access".to_string(),
            params: serde_json::Value::Null,
            op: FirewallOp::GrantTemporary {
                app_id: "org.example.App".to_string(),
                duration: GrantDuration::parse(30_000).unwrap(),
            },
        };
        let desired = request.desired_state(&observed);
        assert!(desired.grant_present);
        assert_eq!(desired.app_id.as_deref(), Some("org.example.App"));
    }

    #[test]
    fn an_option_looking_app_id_is_refused() {
        assert!(validate_app_id("--delete").is_err());
        assert!(validate_app_id("").is_err());
        assert!(validate_app_id("org.example.App").is_ok());
    }
}
