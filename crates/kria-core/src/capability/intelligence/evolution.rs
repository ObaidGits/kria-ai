//! Wave 8 — Capability Evolution Engine (neutral, spec R6 + R29).
//!
//! The Brain continuously reasons over CKB health/benchmarks and PROPOSES
//! evolution actions (upgrade / replace / repair / retire) — never silently
//! swapping a capability the user relies on. Proposals are **auditable**
//! (persisted with a rationale), **reversible**, and **gated by the configured
//! [`AutonomyLevel`]** (R29): elevated/irreversible actions require approval
//! unless the user opted into higher autonomy.
//!
//! Provider-neutral: it reads neutral health/benchmark signals and emits neutral
//! [`EvolutionProposal`]s. It never executes a provider — application of an
//! approved proposal flows through the existing neutral LifecycleManager/platform.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::health::{CapabilityHealth, HealthPolicy, HealthStatus};
use super::REASONING_POLICY_VERSION;
use crate::capability::error::CapError;

/// How autonomously KRIA may act on evolution proposals (spec R29.2).
/// Conservative default: propose-only for elevated/irreversible actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyLevel {
    /// Never act autonomously; only surface proposals when explicitly asked.
    Manual,
    /// Generate proposals but never apply without explicit user approval.
    /// Conservative default: never auto-apply elevated actions.
    #[default]
    ProposeOnly,
    /// Apply low-risk/reversible actions with a notice; propose the rest.
    AutoWithNotice,
    /// Apply autonomously up to elevated risk (still auditable + reversible).
    FullAuto,
}

impl AutonomyLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::ProposeOnly => "propose_only",
            Self::AutoWithNotice => "auto_with_notice",
            Self::FullAuto => "full_auto",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "manual" => Self::Manual,
            "propose_only" | "propose-only" => Self::ProposeOnly,
            "auto_with_notice" | "auto-with-notice" => Self::AutoWithNotice,
            "full_auto" | "full-auto" => Self::FullAuto,
            _ => return None,
        })
    }
}

/// The kind of evolution action proposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    /// Re-acquire a newer version of the same capability.
    Upgrade,
    /// Replace with a different, benchmarked-better capability in the family.
    Replace,
    /// Attempt an in-place repair (reinstall / re-verify).
    Repair,
    /// Retire a chronically-failing / unused capability (reversible archive).
    Retire,
}

impl ProposalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Upgrade => "upgrade",
            Self::Replace => "replace",
            Self::Repair => "repair",
            Self::Retire => "retire",
        }
    }
    /// Whether applying this proposal is an elevated/irreversible-ish action
    /// that must be gated above `AutoWithNotice`.
    pub fn is_elevated(&self) -> bool {
        matches!(self, ProposalKind::Replace | ProposalKind::Retire)
    }
}

/// The lifecycle state of a proposal (spec R29 oversight feed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Approved,
    Applied,
    Rejected,
    Undone,
}

impl ProposalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Applied => "applied",
            Self::Rejected => "rejected",
            Self::Undone => "undone",
        }
    }
}

/// One auditable, reversible evolution proposal (spec R6.2 / R29.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvolutionProposal {
    pub id: String,
    pub kind: ProposalKind,
    /// The capability the proposal is about.
    pub provider_id: String,
    pub capability_id: String,
    /// For Replace: the proposed better capability `(provider_id, capability_id)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<(String, String)>,
    /// Human-readable why-now rationale (explainability, R6.2/R29.1).
    pub rationale: String,
    /// Confidence in the proposal 0.0..=1.0.
    pub confidence: f32,
    /// Whether applying requires approval under the current autonomy level.
    pub requires_approval: bool,
    pub status: ProposalStatus,
    pub policy_version: u32,
    pub created_at: String,
}

/// Neutral durable store for evolution/health/benchmark data. Implemented by the
/// CKB (`SqliteCapabilityKnowledge`) so evolution stays storage-agnostic and the
/// future Memory redesign can re-home it (spec R22). Kept separate from the lean
/// [`CapabilityKnowledge`] trait to avoid bloating the core learned-layer API.
#[async_trait]
pub trait EvolutionStore: Send + Sync {
    /// Per-capability health signals (for scoring + evolution triggers).
    async fn health_snapshots(&self) -> Result<Vec<CapabilityHealth>, CapError>;
    /// Record a benchmark proxy score for a capability.
    async fn record_benchmark(
        &self,
        provider_id: &str,
        capability_id: &str,
        success: bool,
        latency_ms: u64,
        score: f32,
    ) -> Result<(), CapError>;
    /// Mean benchmark score for a capability (`None` if never benchmarked).
    async fn benchmark_score(&self, provider_id: &str, capability_id: &str) -> Option<f32>;
    /// Persist a proposal (auditable history).
    async fn record_proposal(&self, proposal: &EvolutionProposal) -> Result<(), CapError>;
    /// List proposals, newest first, optionally filtered by status.
    async fn list_proposals(
        &self,
        status: Option<ProposalStatus>,
    ) -> Result<Vec<EvolutionProposal>, CapError>;
    /// Update a proposal's status (approve/apply/reject/undo).
    async fn set_proposal_status(&self, id: &str, status: ProposalStatus) -> Result<(), CapError>;
    /// Fetch a single proposal by id.
    async fn get_proposal(&self, id: &str) -> Result<Option<EvolutionProposal>, CapError>;
}

/// Health/benchmark-driven evolution engine (spec R6). Neutral: reads signals
/// from an [`EvolutionStore`], proposes gated reversible actions, persists them.
pub struct DefaultEvolutionEngine<S: EvolutionStore + ?Sized> {
    store: std::sync::Arc<S>,
    health_policy: HealthPolicy,
    autonomy: AutonomyLevel,
}

impl<S: EvolutionStore + ?Sized> DefaultEvolutionEngine<S> {
    pub fn new(store: std::sync::Arc<S>, autonomy: AutonomyLevel) -> Self {
        Self {
            store,
            health_policy: HealthPolicy::default(),
            autonomy,
        }
    }

    pub fn with_health_policy(mut self, policy: HealthPolicy) -> Self {
        self.health_policy = policy;
        self
    }

    /// Whether an action of the given kind may auto-apply under the autonomy
    /// level (R29.3): elevated/irreversible actions need approval unless FullAuto;
    /// non-elevated need at least AutoWithNotice.
    pub fn requires_approval(&self, kind: ProposalKind) -> bool {
        match self.autonomy {
            AutonomyLevel::Manual | AutonomyLevel::ProposeOnly => true,
            AutonomyLevel::AutoWithNotice => kind.is_elevated(),
            AutonomyLevel::FullAuto => false,
        }
    }

    /// Analyze CKB health and produce (and persist) evolution proposals. Pure
    /// proposal generation — never applies. A critical/warning capability with a
    /// healthier, benchmarked in-family alternative yields a Replace; a lone
    /// critical capability yields Repair (then Retire on chronic failure).
    pub async fn analyze(&self) -> Result<Vec<EvolutionProposal>, CapError> {
        let snapshots = self.store.health_snapshots().await?;
        let classified = super::health::classify(&self.health_policy, snapshots);
        let mut proposals = Vec::new();

        for h in &classified {
            if matches!(h.status, HealthStatus::Healthy | HealthStatus::Unknown) {
                continue;
            }
            // Find the healthiest alternative in the same family (excluding self).
            let alt = classified
                .iter()
                .filter(|o| {
                    o.family == h.family
                        && !(o.provider_id == h.provider_id && o.capability_id == h.capability_id)
                        && matches!(o.status, HealthStatus::Healthy)
                })
                .max_by(|a, b| {
                    a.success_rate()
                        .unwrap_or(0.0)
                        .partial_cmp(&b.success_rate().unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

            let (kind, replacement, rationale, confidence) = match (h.status, alt) {
                (HealthStatus::Critical | HealthStatus::Warning, Some(a)) => (
                    ProposalKind::Replace,
                    Some((a.provider_id.clone(), a.capability_id.clone())),
                    format!(
                        "'{}' is {} (success {:.0}%, {} consecutive failures); \
                         healthier in-family alternative '{}' available (success {:.0}%).",
                        h.capability_id,
                        h.status.as_str(),
                        h.success_rate().unwrap_or(0.0) * 100.0,
                        h.consecutive_failures,
                        a.capability_id,
                        a.success_rate().unwrap_or(0.0) * 100.0,
                    ),
                    0.5 + a.success_rate().unwrap_or(0.5) * 0.4,
                ),
                (HealthStatus::Critical, None) => (
                    ProposalKind::Repair,
                    None,
                    format!(
                        "'{}' is critical (success {:.0}%, {} consecutive failures) and no healthy \
                         in-family alternative exists — attempt repair/reinstall.",
                        h.capability_id,
                        h.success_rate().unwrap_or(0.0) * 100.0,
                        h.consecutive_failures,
                    ),
                    0.55,
                ),
                (HealthStatus::Quarantined, _) => (
                    ProposalKind::Retire,
                    None,
                    format!(
                        "'{}' is quarantined (trust/integrity gate) — retire (reversible archive).",
                        h.capability_id
                    ),
                    0.7,
                ),
                _ => continue, // Warning with no alternative → watch, no proposal yet.
            };

            let proposal = EvolutionProposal {
                id: uuid::Uuid::new_v4().to_string(),
                kind,
                provider_id: h.provider_id.clone(),
                capability_id: h.capability_id.clone(),
                replacement,
                rationale,
                confidence: confidence.clamp(0.0, 1.0),
                requires_approval: self.requires_approval(kind),
                status: ProposalStatus::Pending,
                policy_version: REASONING_POLICY_VERSION,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            self.store.record_proposal(&proposal).await?;
            proposals.push(proposal);
        }
        Ok(proposals)
    }

    /// **Really apply** an approved proposal through the neutral
    /// [`LifecycleManager`] (spec R6.2) — the actual capability change, not a
    /// status flip. `Upgrade`/`Repair` do an idempotent re-acquire; `Retire` and
    /// `Replace` archive the failing capability (a Replace's healthier in-family
    /// alternative is already installed and now wins selection). On success the
    /// proposal is marked `Applied`. Provider-neutral: only the `LifecycleManager`
    /// trait is touched, never a provider.
    pub async fn apply(
        &self,
        proposal: &EvolutionProposal,
        lifecycle: &dyn super::LifecycleManager,
    ) -> Result<(), CapError> {
        match proposal.kind {
            ProposalKind::Upgrade | ProposalKind::Repair => {
                lifecycle
                    .upgrade(&proposal.provider_id, &proposal.capability_id)
                    .await?;
            }
            ProposalKind::Retire | ProposalKind::Replace => {
                lifecycle
                    .retire(&proposal.provider_id, &proposal.capability_id)
                    .await?;
            }
        }
        self.store
            .set_proposal_status(&proposal.id, ProposalStatus::Applied)
            .await?;
        Ok(())
    }

    /// **Really undo** an applied proposal (spec R6.2 / R29.1 reversibility):
    /// `Retire`/`Replace` recover the archived capability; `Upgrade`/`Repair`
    /// have no version history to revert, so undo is an honest status-only
    /// reversal (documented limitation, not silent). Marks the proposal `Undone`.
    pub async fn undo(
        &self,
        proposal: &EvolutionProposal,
        lifecycle: &dyn super::LifecycleManager,
    ) -> Result<(), CapError> {
        if matches!(proposal.kind, ProposalKind::Retire | ProposalKind::Replace) {
            lifecycle
                .recover(&proposal.provider_id, &proposal.capability_id)
                .await?;
        }
        self.store
            .set_proposal_status(&proposal.id, ProposalStatus::Undone)
            .await?;
        Ok(())
    }

    /// Analyze + **autonomously apply** the proposals the autonomy level permits
    /// (spec R29.3): only proposals with `requires_approval == false`
    /// (AutoWithNotice for low-risk, FullAuto for all) are applied; the rest
    /// remain pending for user approval. Returns the applied proposals.
    pub async fn auto_apply(
        &self,
        lifecycle: &dyn super::LifecycleManager,
    ) -> Result<Vec<EvolutionProposal>, CapError> {
        let proposals = self.analyze().await?;
        let mut applied = Vec::new();
        for p in proposals {
            if !p.requires_approval && self.apply(&p, lifecycle).await.is_ok() {
                applied.push(p);
            }
        }
        Ok(applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeStore {
        snapshots: Vec<CapabilityHealth>,
        proposals: Mutex<Vec<EvolutionProposal>>,
    }

    #[async_trait]
    impl EvolutionStore for FakeStore {
        async fn health_snapshots(&self) -> Result<Vec<CapabilityHealth>, CapError> {
            Ok(self.snapshots.clone())
        }
        async fn record_benchmark(
            &self,
            _p: &str,
            _c: &str,
            _s: bool,
            _l: u64,
            _sc: f32,
        ) -> Result<(), CapError> {
            Ok(())
        }
        async fn benchmark_score(&self, _p: &str, _c: &str) -> Option<f32> {
            None
        }
        async fn record_proposal(&self, p: &EvolutionProposal) -> Result<(), CapError> {
            self.proposals.lock().unwrap().push(p.clone());
            Ok(())
        }
        async fn list_proposals(
            &self,
            _s: Option<ProposalStatus>,
        ) -> Result<Vec<EvolutionProposal>, CapError> {
            Ok(self.proposals.lock().unwrap().clone())
        }
        async fn set_proposal_status(&self, _id: &str, _s: ProposalStatus) -> Result<(), CapError> {
            Ok(())
        }
        async fn get_proposal(&self, _id: &str) -> Result<Option<EvolutionProposal>, CapError> {
            Ok(None)
        }
    }

    fn snap(cap: &str, family: &str, total: u64, succ: u64, consec: u32) -> CapabilityHealth {
        CapabilityHealth {
            provider_id: "p".into(),
            capability_id: cap.into(),
            family: family.into(),
            total,
            successes: succ,
            consecutive_failures: consec,
            last_latency_ms: Some(10),
            last_failure: None,
            quarantined: false,
            status: HealthStatus::Unknown,
        }
    }

    #[tokio::test]
    async fn critical_with_alternative_proposes_replace() {
        let store = Arc::new(FakeStore {
            snapshots: vec![
                snap("bad_ocr", "Ocr", 10, 2, 4),   // critical
                snap("good_ocr", "Ocr", 20, 19, 0), // healthy alternative
            ],
            proposals: Mutex::new(vec![]),
        });
        let eng = DefaultEvolutionEngine::new(store, AutonomyLevel::ProposeOnly);
        let props = eng.analyze().await.unwrap();
        let replace = props
            .iter()
            .find(|p| p.kind == ProposalKind::Replace)
            .expect("replace");
        assert_eq!(replace.capability_id, "bad_ocr");
        assert_eq!(replace.replacement.as_ref().unwrap().1, "good_ocr");
        assert!(
            replace.requires_approval,
            "propose_only must gate application"
        );
    }

    #[tokio::test]
    async fn critical_without_alternative_proposes_repair() {
        let store = Arc::new(FakeStore {
            snapshots: vec![snap("lonely", "Pdf", 10, 1, 5)],
            proposals: Mutex::new(vec![]),
        });
        let eng = DefaultEvolutionEngine::new(store, AutonomyLevel::ProposeOnly);
        let props = eng.analyze().await.unwrap();
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].kind, ProposalKind::Repair);
    }

    #[tokio::test]
    async fn healthy_produces_no_proposal() {
        let store = Arc::new(FakeStore {
            snapshots: vec![snap("fine", "Data", 50, 49, 0)],
            proposals: Mutex::new(vec![]),
        });
        let eng = DefaultEvolutionEngine::new(store, AutonomyLevel::ProposeOnly);
        assert!(eng.analyze().await.unwrap().is_empty());
    }

    #[test]
    fn autonomy_gating() {
        let store = Arc::new(FakeStore::default());
        let manual = DefaultEvolutionEngine::new(store.clone(), AutonomyLevel::Manual);
        assert!(manual.requires_approval(ProposalKind::Upgrade));
        let notice = DefaultEvolutionEngine::new(store.clone(), AutonomyLevel::AutoWithNotice);
        assert!(!notice.requires_approval(ProposalKind::Upgrade)); // low-risk auto
        assert!(notice.requires_approval(ProposalKind::Replace)); // elevated gated
        let full = DefaultEvolutionEngine::new(store, AutonomyLevel::FullAuto);
        assert!(!full.requires_approval(ProposalKind::Replace));
    }
}
