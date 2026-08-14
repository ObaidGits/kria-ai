//! System health: diagnosis, log queries, and recovery recipes.
//!
//! linux-os-control-production task **4.6** (OSC-022).
//!
//! # The worst possible answer this domain can give
//!
//! "Healthy" because a check failed. A diagnosis that cannot reach a subsystem
//! must report **undetermined** for it, never a pass — a user acting on a false
//! all-clear stops looking for the actual fault. Every check therefore has three
//! outcomes, not two.
//!
//! # Logs are sensitive, not public
//!
//! The journal carries authentication failures, tokens applications logged by
//! mistake, and other users' activity. So `get_system_logs` is RED, the query is
//! **scoped and bounded**, and there is no way to ask for "everything".
//!
//! # A recovery recipe is a closed set, never a script
//!
//! `run_recovery_recipe` takes a *recipe id* that must already exist in-tree, plus
//! the plan digest the caller reviewed. A caller cannot supply steps. Accepting a
//! recipe body would be arbitrary privileged execution wearing a helpful name, and
//! the steps are the whole reason this operation is RED with per-step verification.

use async_trait::async_trait;

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
pub const HEALTH_PROVIDER_ID: &str = "system-health";

/// Largest number of log lines a single query may return.
pub const MAX_LOG_LINES: u32 = 500;

/// Longest log window a single query may span.
pub const MAX_LOG_WINDOW_HOURS: u32 = 24;

/// A health check's verdict.
///
/// Three-valued on purpose: collapsing `Undetermined` into `Unhealthy` cries wolf,
/// and collapsing it into `Healthy` hides a real fault. Neither is acceptable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthVerdict {
    /// The check passed.
    Healthy,
    /// The check failed.
    Unhealthy,
    /// The check could not be performed.
    Undetermined,
}

impl HealthVerdict {
    /// A stable token for reporting.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Unhealthy => "unhealthy",
            Self::Undetermined => "undetermined",
        }
    }
}

/// Which subsystem a diagnosis covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HealthDomain {
    /// Disk capacity and filesystem errors.
    Storage,
    /// Memory pressure.
    Memory,
    /// Failed services.
    Services,
    /// Thermal state.
    Thermal,
    /// Network reachability.
    Network,
}

impl HealthDomain {
    /// Every domain, in a stable order.
    #[must_use]
    pub fn all() -> &'static [HealthDomain] {
        &[
            Self::Memory,
            Self::Network,
            Self::Services,
            Self::Storage,
            Self::Thermal,
        ]
    }

    /// A stable token.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Storage => "storage",
            Self::Memory => "memory",
            Self::Services => "services",
            Self::Thermal => "thermal",
            Self::Network => "network",
        }
    }

    /// Parse a caller-supplied scope. An unknown token is refused rather than
    /// silently widened to "everything".
    pub fn parse(raw: &str) -> Result<Self, OsControlError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "storage" | "disk" => Ok(Self::Storage),
            "memory" | "ram" => Ok(Self::Memory),
            "services" => Ok(Self::Services),
            "thermal" | "temperature" => Ok(Self::Thermal),
            "network" => Ok(Self::Network),
            _ => Err(OsControlError::InvalidRequest {
                field: SafeField::new("scope"),
                reason: SafeText::new(
                    "scope must be one of storage, memory, services, thermal, network",
                ),
            }),
        }
    }
}

/// One subsystem's finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthFinding {
    /// The subsystem.
    pub domain: HealthDomain,
    /// The verdict.
    pub verdict: HealthVerdict,
    /// A bounded, redacted detail line.
    pub detail: Option<SafeText>,
}

/// A full diagnosis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthReport {
    /// Per-subsystem findings, in a stable order.
    pub findings: Vec<HealthFinding>,
}

impl HealthReport {
    /// The overall verdict.
    ///
    /// `Unhealthy` if anything failed; otherwise `Undetermined` if anything could
    /// not be checked; only `Healthy` when every check actually passed. The order
    /// matters: an all-clear must require real evidence from every check.
    #[must_use]
    pub fn overall(&self) -> HealthVerdict {
        if self
            .findings
            .iter()
            .any(|f| f.verdict == HealthVerdict::Unhealthy)
        {
            HealthVerdict::Unhealthy
        } else if self
            .findings
            .iter()
            .any(|f| f.verdict == HealthVerdict::Undetermined)
        {
            HealthVerdict::Undetermined
        } else if self.findings.is_empty() {
            // No checks ran at all, so nothing is known.
            HealthVerdict::Undetermined
        } else {
            HealthVerdict::Healthy
        }
    }
}

/// A bounded, scoped log query. There is deliberately no "all logs" form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogQuery {
    /// The systemd unit to scope to, when given.
    pub unit: Option<String>,
    /// How far back to look, in hours.
    pub since_hours: u32,
    /// Maximum lines to return.
    pub max_lines: u32,
    /// Minimum priority (0=emerg .. 7=debug).
    pub max_priority: u8,
}

impl LogQuery {
    /// Validate and bound a query.
    pub fn parse(
        unit: Option<&str>,
        since_hours: u32,
        max_lines: u32,
        max_priority: u8,
    ) -> Result<Self, OsControlError> {
        let unit = match unit.map(str::trim).filter(|u| !u.is_empty()) {
            Some(unit) => {
                let ok = unit.len() <= 128
                    && !unit.starts_with('-')
                    && unit
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'));
                if !ok {
                    return Err(OsControlError::InvalidRequest {
                        field: SafeField::new("unit"),
                        reason: SafeText::new("unit is not a valid systemd unit name"),
                    });
                }
                Some(unit.to_string())
            }
            None => None,
        };
        if since_hours == 0 || since_hours > MAX_LOG_WINDOW_HOURS {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("since_hours"),
                reason: SafeText::new("since_hours must be between 1 and 24"),
            });
        }
        if max_lines == 0 || max_lines > MAX_LOG_LINES {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("max_lines"),
                reason: SafeText::new("max_lines must be between 1 and 500"),
            });
        }
        if max_priority > 7 {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("max_priority"),
                reason: SafeText::new("max_priority must be between 0 and 7"),
            });
        }
        Ok(Self {
            unit,
            since_hours,
            max_lines,
            max_priority,
        })
    }
}

/// One returned log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// Unix timestamp in seconds.
    pub timestamp: u64,
    /// The unit that emitted it.
    pub unit: Option<String>,
    /// Syslog priority.
    pub priority: u8,
    /// The message, bounded and redacted.
    pub message: SafeText,
}

/// A bounded page of log lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPage {
    /// The lines.
    pub lines: Vec<LogLine>,
    /// Whether the query hit its line bound.
    pub truncated: bool,
}

/// A recovery recipe's identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecoveryRecipeId(String);

impl RecoveryRecipeId {
    /// Validate a recipe id.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, OsControlError> {
        let raw = raw.as_ref().trim();
        let ok = !raw.is_empty()
            && raw.len() <= 64
            && raw
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("recipe_id"),
                reason: SafeText::new("recipe_id must be a lowercase in-tree recipe id"),
            });
        }
        Ok(Self(raw.to_string()))
    }

    /// Borrow the id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One reviewed recovery step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryStep {
    /// A stable step id.
    pub step: String,
    /// Whether this step can be compensated if a later step fails.
    pub compensable: bool,
}

/// A reviewed in-tree recovery recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryRecipe {
    /// The recipe id.
    pub recipe: RecoveryRecipeId,
    /// Its ordered steps.
    pub steps: Vec<RecoveryStep>,
    /// The definition revision.
    pub revision: u64,
}

impl RecoveryRecipe {
    /// The digest a caller must have reviewed.
    #[must_use]
    pub fn plan_digest(&self) -> String {
        let steps: Vec<String> = self
            .steps
            .iter()
            .map(|s| format!("{}:{}", s.step, s.compensable))
            .collect();
        Digest::of_str(&format!(
            "recipe:{}:{}:{}",
            self.recipe.as_str(),
            self.revision,
            steps.join(",")
        ))
        .as_hex()
        .to_string()
    }
}

/// The in-tree recipe registry. Empty until a recipe is reviewed in — a caller can
/// never add one at call time.
pub const IN_TREE_RECIPES: &[RecoveryRecipe] = &[];

/// A normalized health observation, used only by the recipe lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthState {
    /// The recipe this observation is about.
    pub recipe: String,
    /// Whether the recipe has been applied.
    pub applied: bool,
}

impl NormalizedObservation for HealthState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!("health-recipe:{}:{}", self.recipe, self.applied))
    }
}

/// One governed recipe request.
#[derive(Debug, Clone)]
pub struct HealthRequest {
    /// The canonical tool/action name.
    pub action: String,
    /// The canonical tool parameters.
    pub params: serde_json::Value,
    /// The recipe to run.
    pub recipe: RecoveryRecipeId,
    /// The plan digest the caller reviewed.
    pub expected_plan_digest: String,
}

impl HealthRequest {
    /// The state this request is trying to reach.
    #[must_use]
    pub fn desired_state(&self) -> HealthState {
        HealthState {
            recipe: self.recipe.as_str().to_string(),
            applied: true,
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
pub trait HealthTransport: Send + Sync {
    /// The provider identity.
    fn provider_id(&self) -> ProviderId;

    /// Diagnose one or all subsystems.
    async fn diagnose(
        &self,
        ctx: &HostExecutionContext,
        scope: Option<HealthDomain>,
    ) -> Result<HealthReport, OsControlError>;

    /// Run a bounded log query.
    async fn query_logs(
        &self,
        ctx: &HostExecutionContext,
        query: &LogQuery,
    ) -> Result<LogPage, OsControlError>;

    /// Whether a recipe has already been applied in this session.
    async fn read_recipe_applied(
        &self,
        ctx: &HostExecutionContext,
        recipe: &RecoveryRecipeId,
    ) -> Result<bool, OsControlError>;

    /// Run a reviewed recipe.
    async fn run_recipe(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        recipe: &RecoveryRecipe,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The governed provider.
pub struct SystemHealthControl<T: HealthTransport> {
    transport: T,
    registry: &'static [RecoveryRecipe],
}

impl<T: HealthTransport> SystemHealthControl<T> {
    /// Compose over a transport, using the in-tree recipe registry.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            registry: IN_TREE_RECIPES,
        }
    }

    /// Compose with an explicit registry (tests supply their own).
    #[must_use]
    pub fn with_registry(transport: T, registry: &'static [RecoveryRecipe]) -> Self {
        Self {
            transport,
            registry,
        }
    }

    /// Borrow the transport.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Resolve a recipe from the in-tree registry.
    ///
    /// A recipe that is not in-tree is refused: a caller supplies an id, never a
    /// definition, so no new sequence of privileged steps can appear at call time.
    pub fn resolve_recipe(&self, id: &RecoveryRecipeId) -> Result<&RecoveryRecipe, OsControlError> {
        self.registry
            .iter()
            .find(|r| r.recipe == *id)
            .ok_or_else(|| OsControlError::Unsupported {
                capability: CapabilityId::new("run_recovery_recipe"),
                reason: SafeText::new(
                    "no such reviewed in-tree recovery recipe; a recipe body cannot be supplied by \
                     the caller",
                ),
            })
    }

    fn satisfying(&self, observed: &HealthState) -> SatisfyingVerification<HealthState> {
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
impl<T: HealthTransport> DesiredStateControl<HealthRequest, HealthState>
    for SystemHealthControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &HealthRequest,
    ) -> Result<HealthState, OsControlError> {
        let applied = self
            .transport
            .read_recipe_applied(ctx, &request.recipe)
            .await?;
        Ok(HealthState {
            recipe: request.recipe.as_str().to_string(),
            applied,
        })
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &HealthRequest,
        _desired: &HealthState,
    ) -> Result<ApplyOutcome, OsControlError> {
        let recipe = self.resolve_recipe(&request.recipe)?;
        // The reviewed digest must still match. A recipe edited since the caller
        // read it is a different set of privileged steps than the one approved.
        if recipe.plan_digest() != request.expected_plan_digest {
            return Err(OsControlError::GrantInvalid {
                reason: crate::os_control::error::GrantInvalidReason::StaleSnapshot,
            });
        }
        self.transport.run_recipe(ctx, recipe).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &HealthRequest,
        desired: &HealthState,
    ) -> Result<VerificationReport<HealthState>, OsControlError> {
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
        // The contract is CompensationOnly: individual steps compensate as the
        // recipe unwinds, and there is no single inverse for the whole run.
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("health.rollback"),
            reason: SafeText::new(
                "a recovery recipe compensates per step as it unwinds; there is no whole-run inverse",
            ),
        })
    }
}

/// The port a handler resolves.
#[async_trait]
pub trait SystemHealthControlPort: DesiredStateControl<HealthRequest, HealthState> {
    /// Diagnose one or all subsystems.
    async fn diagnose(
        &self,
        ctx: &HostExecutionContext,
        scope: Option<HealthDomain>,
    ) -> Result<HealthReport, OsControlError>;

    /// Run a bounded log query.
    async fn logs(
        &self,
        ctx: &HostExecutionContext,
        query: &LogQuery,
    ) -> Result<LogPage, OsControlError>;
}

#[async_trait]
impl<T: HealthTransport> SystemHealthControlPort for SystemHealthControl<T> {
    async fn diagnose(
        &self,
        ctx: &HostExecutionContext,
        scope: Option<HealthDomain>,
    ) -> Result<HealthReport, OsControlError> {
        self.transport.diagnose(ctx, scope).await
    }

    async fn logs(
        &self,
        ctx: &HostExecutionContext,
        query: &LogQuery,
    ) -> Result<LogPage, OsControlError> {
        self.transport.query_logs(ctx, query).await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn finding(domain: HealthDomain, verdict: HealthVerdict) -> HealthFinding {
        HealthFinding {
            domain,
            verdict,
            detail: None,
        }
    }

    #[test]
    fn an_undetermined_check_never_yields_an_overall_healthy() {
        // The central rule: a failed check must not become an all-clear.
        let report = HealthReport {
            findings: vec![
                finding(HealthDomain::Memory, HealthVerdict::Healthy),
                finding(HealthDomain::Storage, HealthVerdict::Undetermined),
            ],
        };
        assert_eq!(report.overall(), HealthVerdict::Undetermined);
    }

    #[test]
    fn any_unhealthy_dominates() {
        let report = HealthReport {
            findings: vec![
                finding(HealthDomain::Memory, HealthVerdict::Undetermined),
                finding(HealthDomain::Storage, HealthVerdict::Unhealthy),
            ],
        };
        assert_eq!(report.overall(), HealthVerdict::Unhealthy);
    }

    #[test]
    fn healthy_requires_every_check_to_have_actually_passed() {
        let all_good = HealthReport {
            findings: HealthDomain::all()
                .iter()
                .map(|d| finding(*d, HealthVerdict::Healthy))
                .collect(),
        };
        assert_eq!(all_good.overall(), HealthVerdict::Healthy);

        // No checks ran, so nothing is known — not "healthy".
        let empty = HealthReport { findings: vec![] };
        assert_eq!(empty.overall(), HealthVerdict::Undetermined);
    }

    #[test]
    fn log_queries_are_bounded_and_scoped() {
        assert!(LogQuery::parse(None, 0, 100, 6).is_err(), "zero window");
        assert!(LogQuery::parse(None, 999, 100, 6).is_err(), "window too wide");
        assert!(LogQuery::parse(None, 1, 0, 6).is_err(), "zero lines");
        assert!(LogQuery::parse(None, 1, 9999, 6).is_err(), "too many lines");
        assert!(LogQuery::parse(None, 1, 100, 9).is_err(), "bad priority");
        assert!(LogQuery::parse(Some("--vacuum"), 1, 100, 6).is_err(), "option-looking unit");
        assert!(LogQuery::parse(Some("NetworkManager.service"), 2, 200, 4).is_ok());
    }

    #[test]
    fn an_unknown_diagnosis_scope_is_refused_not_widened() {
        assert!(HealthDomain::parse("everything").is_err());
        assert_eq!(HealthDomain::parse("disk").unwrap(), HealthDomain::Storage);
    }

    #[test]
    fn a_recipe_not_in_the_registry_is_refused() {
        struct T;
        #[async_trait]
        impl HealthTransport for T {
            fn provider_id(&self) -> ProviderId {
                ProviderId::new("t")
            }
            async fn diagnose(
                &self,
                _c: &HostExecutionContext,
                _s: Option<HealthDomain>,
            ) -> Result<HealthReport, OsControlError> {
                unreachable!()
            }
            async fn query_logs(
                &self,
                _c: &HostExecutionContext,
                _q: &LogQuery,
            ) -> Result<LogPage, OsControlError> {
                unreachable!()
            }
            async fn read_recipe_applied(
                &self,
                _c: &HostExecutionContext,
                _r: &RecoveryRecipeId,
            ) -> Result<bool, OsControlError> {
                unreachable!()
            }
            async fn run_recipe(
                &self,
                _c: &AdmittedMutationContext<'_>,
                _r: &RecoveryRecipe,
            ) -> Result<ApplyOutcome, OsControlError> {
                unreachable!()
            }
        }
        let control = SystemHealthControl::new(T);
        let id = RecoveryRecipeId::parse("clear-thumbnail-cache").unwrap();
        // A caller supplies an id, never a body — an unknown id cannot run.
        assert!(control.resolve_recipe(&id).is_err());
    }

    #[test]
    fn a_recipe_id_must_be_a_plain_in_tree_token() {
        assert!(RecoveryRecipeId::parse("Clear Cache").is_err());
        assert!(RecoveryRecipeId::parse("").is_err());
        assert!(RecoveryRecipeId::parse("clear-cache-1").is_ok());
    }
}
