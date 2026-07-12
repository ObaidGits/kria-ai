//! Capability-intelligence layer — KRIA's *engineering brain* over capabilities.
//!
//! This is the neutral home for everything that turns a goal into a **Solution
//! Plan**: capability knowledge (learned), goal classification, strategy
//! generation, confidence + cost reasoning, composition, lifecycle, evolution,
//! and synthesis. It depends ONLY on the neutral capability types
//! ([`CapabilityDescriptor`], [`Effects`], provider traits) — never on a
//! provider-native type (Brain/Hands invariant, spec R23).
//!
//! # Status (Wave 0 — P0 seams)
//!
//! This module currently defines the **stable neutral vocabulary** the later
//! waves implement against: capability [`CapabilityKind`] / [`CapabilityFamily`]
//! (P0.4), the versioned [`ReasoningPolicy`] + telemetry schema (P0.6), the
//! neutral plan/decision value types, and the component **traits** (P0.3). The
//! traits have no production implementations yet; each is landed and wired in its
//! phase (CKB → Reasoner → Planner → Lifecycle → ...). Everything here is inert
//! until the corresponding [`CapabilityIntelligenceConfig`] flag is enabled, so
//! flag-off parity holds (spec Property 1).
//!
//! [`CapabilityDescriptor`]: super::descriptor::CapabilityDescriptor
//! [`Effects`]: super::descriptor::Effects
//! [`CapabilityIntelligenceConfig`]: super::config::CapabilityIntelligenceConfig

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::descriptor::CapabilityDescriptor;
use super::error::CapError;
use super::provider::RequestContext;

pub mod arbitration;
pub mod arg_gen;
pub mod benchmark;
pub mod capability_graph;
pub mod discovery;
pub mod evolution;
pub mod health;
pub mod jobs;
pub mod kind;
pub mod knowledge;
pub mod lifecycle;
pub mod llm_proposer;
pub mod marketplace;
#[cfg(test)]
mod neutrality;
pub mod plan_executor;
pub mod plan_permission;
pub mod planner;
pub mod primitives;
pub mod reliability;
pub mod selector;
pub mod synthesis;

pub use arbitration::{
    ArbitrationDecision, ArbitrationPolicy, DomainEvidence, DomainScore, PlanningAuthority,
    PlanningDomain,
};
pub use arg_gen::{
    generate_arguments, schema_expects_arguments, validate_against_schema, DefaultArgumentGenerator,
};
pub use benchmark::{
    select_in_family, BenchmarkExecutor, BenchmarkResult, DefaultBenchmarkHarness,
    FamilyTradeoffWeights, GoldenCase,
};
pub use capability_graph::{
    CapabilityGraph, CodeRunner, GraphEdge, GraphNode, NodeExecutor, NodeOp, IR_SCHEMA_VERSION,
    PRIMITIVE_SET_VERSION,
};
pub use discovery::{ContinuousDiscoveryEngine, DiscoveryPolicy, DiscoveryReport, DiscoveryStatus};
pub use evolution::{
    AutonomyLevel, DefaultEvolutionEngine, EvolutionProposal, EvolutionStore, ProposalKind,
    ProposalStatus,
};
pub use health::{CapabilityHealth, HealthPolicy, HealthStatus};
pub use jobs::{redact_secrets, Job, JobManager, JobState, JobStore};
pub use kind::{infer_family, infer_kind, CapabilityFamily, CapabilityKind};
pub use knowledge::{SqliteCapabilityKnowledge, CKB_SCHEMA_VERSION};
pub use lifecycle::DefaultLifecycleManager;
pub use llm_proposer::{
    build_prompt, parse_code_proposal, parse_pipeline, LlmIrProposer, TextGenerator,
    IR_PROMPT_VERSION,
};
pub use marketplace::{
    trust_tier_rank, version_satisfies, ArtifactVerifier, CapabilityCoordinate, CatalogCache,
    CatalogRanker, CatalogRankingPolicy, ClawHubListing, DependencySpec, Digest, DigestAlgorithm,
    IntegrityVerdict, PublishedVersion, Quarantine, RankedCatalogEntry, Rating, Review, Signature,
    TrustPolicy, TrustVerdict, UpdateChannel,
};
pub use plan_executor::{CapabilityPlanExecutor, PlanRunResult, StepRun};
pub use plan_permission::{authorize_plan, plan_key};
pub use planner::DefaultCapabilityPlanner;
pub use reliability::{classify, FailureClass, RetryPolicy};
pub use selector::DefaultCapabilitySelector;
pub use synthesis::{
    propose_validated, CapabilityGapAnalyzer, CapabilitySpecification, DeterministicIrProposer,
    GapResolution, IrProposer,
};

/// Version of the reasoning policy (weights, thresholds, priors). Recorded in
/// every reasoning trace so behavior changes are reproducible + A/B-testable
/// (spec R24.2). Bump on any change to the decision math.
pub const REASONING_POLICY_VERSION: u32 = 1;

/// Version of the capability telemetry / event schema (spec R24.2). Bump on any
/// change to the shape of `capability:*` events / trace records.
pub const TELEMETRY_SCHEMA_VERSION: u32 = 1;

/// Tunable reasoning policy — data, not code. Weights + thresholds that drive
/// candidate comparison, the native-first sufficiency gate, and the reasoning
/// budget. Serialized into the trace by `version`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningPolicy {
    /// Policy version stamped into traces (spec R24.2).
    pub version: u32,
    /// Confidence at/above which a single-capability match takes the fast path
    /// and skips deep reasoning (spec R2.6).
    pub fast_path_confidence: f32,
    /// Confidence at/above which a native/installed candidate is deemed
    /// *sufficient* and remote marketplace/generation is skipped (spec R3.6).
    pub native_sufficiency_confidence: f32,
    /// Minimum confidence below which the reasoner asks a clarifying question or
    /// declines instead of acting (spec R2.3 / R3.3).
    pub min_action_confidence: f32,
    /// Max reasoning rounds in the iterative loop (spec R2.5/R2.7).
    pub max_rounds: u32,
    /// Max candidates evaluated per round (budget bound, spec R2.7).
    pub max_candidates: usize,
    /// Wall-time budget in milliseconds for deep reasoning (spec R2.7).
    pub budget_ms: u64,
    /// Relative weights for multi-attribute candidate scoring (spec R3.5/R15).
    pub weight_semantic: f32,
    pub weight_lexical: f32,
    pub weight_success: f32,
    pub weight_trust: f32,
    pub weight_cost: f32,
    pub weight_recency: f32,
}

impl Default for ReasoningPolicy {
    fn default() -> Self {
        Self {
            version: REASONING_POLICY_VERSION,
            fast_path_confidence: 0.80,
            native_sufficiency_confidence: 0.70,
            min_action_confidence: 0.45,
            max_rounds: 3,
            max_candidates: 12,
            budget_ms: 4_000,
            weight_semantic: 0.45,
            weight_lexical: 0.15,
            weight_success: 0.15,
            weight_trust: 0.10,
            weight_cost: 0.10,
            weight_recency: 0.05,
        }
    }
}

/// Learned/semantic classification of a user goal (spec R2.2). Open vocabulary
/// via `Other` so new goal-classes need no code change; the common classes are
/// named for policy/telemetry ergonomics only, NOT keyword routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalClass {
    Information,
    Analysis,
    Transformation,
    Automation,
    Generation,
    Coding,
    Desktop,
    Research,
    Vision,
    /// Any goal-class not in the named set (open vocabulary).
    Other(String),
}

/// The estimated cost of running a candidate/strategy (spec R15). Populated by a
/// [`CostModel`], calibrated from CKB performance history (spec R30.1).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CostVector {
    pub latency_ms: Option<u64>,
    pub gpu_mb: Option<u64>,
    pub ram_mb: Option<u64>,
    pub tokens: Option<u64>,
    /// Monetary cost estimate in micro-USD (avoids float money).
    pub money_micros: Option<u64>,
    pub install_cost_ms: Option<u64>,
    /// Qualitative maintenance burden 0.0..=1.0.
    pub maintenance: Option<f32>,
    /// True when values are conservative defaults (not yet calibrated, R30.1).
    pub uncalibrated: bool,
}

/// A scored candidate capability for a need (spec R3.1). Component signals are
/// kept for transparency + the reasoning trace.
#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub descriptor: CapabilityDescriptor,
    pub kind: CapabilityKind,
    pub family: CapabilityFamily,
    pub semantic: f32,
    pub lexical: f32,
    pub learned_success: f32,
    pub trust: f32,
    pub recency: f32,
    pub cost: CostVector,
    /// Fused, calibrated confidence 0.0..=1.0.
    pub confidence: f32,
}

/// The path the reasoner chose for a goal/need (spec R3.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPath {
    /// Reuse an already-installed capability (native or installed).
    Reuse,
    /// Use a native tool.
    Native,
    /// Compose multiple capabilities into a plan.
    Compose,
    /// Search + install from a marketplace, then execute.
    Acquire,
    /// Generate (synthesize) a new capability.
    Generate,
    /// Insufficient confidence — ask the user a clarifying question.
    Ask,
}

/// The outcome of confidence-based candidate comparison + path selection
/// (spec R3): the ranked candidates, the chosen one (if any), the calibrated
/// confidence, the execution path, and a human-readable rationale for the trace.
#[derive(Debug, Clone)]
pub struct Selection {
    /// Candidates, best first.
    pub candidates: Vec<ScoredCandidate>,
    /// The chosen `(provider_id, capability_id)`, or `None` for Ask/Abstain.
    pub chosen: Option<(String, String)>,
    /// Calibrated confidence of the top candidate (0.0..=1.0).
    pub confidence: f32,
    /// The chosen execution path.
    pub path: ExecutionPath,
    /// Why this path/candidate (for the reasoning trace + decision record).
    pub rationale: String,
    /// Policy version that produced this selection (spec R24.2).
    pub policy_version: u32,
}

/// A candidate solution strategy the [`StrategyGenerator`] produced (spec R2.4),
/// carrying its estimated confidence/risk/cost/reuse value before a concrete
/// plan is built.
#[derive(Debug, Clone)]
pub struct Strategy {
    pub label: String,
    pub path: ExecutionPath,
    pub confidence: f32,
    /// Risk 0.0..=1.0 (higher = riskier).
    pub risk: f32,
    pub cost: CostVector,
    /// Estimated future-reuse value 0.0..=1.0.
    pub reuse_value: f32,
    pub rationale: String,
}

/// One step of a [`SolutionPlan`] (spec R4, saga-structured with compensation).
#[derive(Debug, Clone)]
pub struct PlanStep {
    pub provider_id: String,
    pub capability_id: String,
    pub args: serde_json::Value,
    /// Step indices whose outputs feed this step's inputs (typed IO, R4.4).
    pub inputs_from: Vec<usize>,
    /// Optional compensation/rollback action for saga safety (R4.3).
    pub compensation: Option<String>,
    pub timeout_ms: Option<u64>,
    /// Per-step confidence (spec R2.10).
    pub confidence: f32,
}

/// A composed solution for a goal — one or many capabilities as an execution
/// graph, emitted into the existing HTN runtime for execution (spec R4.2).
#[derive(Debug, Clone)]
pub struct SolutionPlan {
    pub goal_class: GoalClass,
    pub path: ExecutionPath,
    pub steps: Vec<PlanStep>,
    /// Union of step effects at max risk (plan-level permission, spec R11.1).
    pub plan_effects: Vec<super::descriptor::Effect>,
    /// Plan-level reversibility at max risk: `Irreversible` if ANY step is
    /// irreversible (so the whole plan is permissioned conservatively, R11.1).
    pub plan_reversibility: super::descriptor::Reversibility,
    pub confidence: f32,
    pub rationale: String,
    /// Reasoning budget consumed producing this plan (ms).
    pub budget_used_ms: u64,
    /// Policy version that produced this plan (spec R24.2).
    pub policy_version: u32,
}

/// A durable record of one engineering decision — why a path/capability was
/// chosen and why alternatives were rejected (spec R16, powers explainability).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub id: String,
    pub goal: String,
    pub goal_class: GoalClass,
    /// `(provider_id, capability_id, confidence)` of considered candidates.
    pub candidates: Vec<(String, String, f32)>,
    pub chosen: Option<(String, String)>,
    /// `(provider_id, capability_id, reason)` for rejected candidates.
    pub rejected: Vec<(String, String, String)>,
    pub path: ExecutionPath,
    pub confidence: f32,
    pub policy_version: u32,
    pub created_at: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Component traits (P0.3). Production implementations land in their phases.
// All are neutral: no provider-native types, no hardcoded provider names.
// ─────────────────────────────────────────────────────────────────────────────

/// P1 — durable, learned knowledge about capabilities (usage, outcomes, health,
/// provenance, relationships, preferences, decision records). Storage-agnostic
/// so the future global Memory redesign can re-home it (spec R1/R22).
#[async_trait]
pub trait CapabilityKnowledge: Send + Sync {
    /// Record that a capability was installed/became known.
    async fn record_install(&self, descriptor: &CapabilityDescriptor) -> Result<(), CapError>;
    /// Record one execution outcome (+ optional latency/failure explanation).
    async fn record_outcome(
        &self,
        provider_id: &str,
        capability_id: &str,
        ok: bool,
        latency_ms: Option<u64>,
        failure: Option<&str>,
    ) -> Result<(), CapError>;
    /// Record an engineering decision (spec R16).
    async fn record_decision(&self, decision: &DecisionRecord) -> Result<(), CapError>;
    /// The installed/known capabilities (grounding source, spec R1.3).
    async fn list_installed(&self) -> Result<Vec<CapabilityDescriptor>, CapError>;
    /// Learned success rate for a capability (0.5 when unobserved).
    async fn success_rate(&self, provider_id: &str, capability_id: &str) -> f32;
    /// Purge all knowledge for a capability (cascade delete, spec R1.6).
    async fn purge(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError>;
    /// Set the lifecycle state of a capability (e.g. `enabled`/`archived`) —
    /// powers reversible retirement (spec R19). Unknown capability ⇒ no-op.
    async fn set_state(
        &self,
        provider_id: &str,
        capability_id: &str,
        state: &str,
    ) -> Result<(), CapError>;
    /// Stable schema version for migration negotiation (spec R22).
    fn schema_version(&self) -> u32;
}

/// A source of candidate capabilities for a need (native / installed / catalog /
/// synthesizing). Uniform contribution across kinds (spec R3.1).
#[async_trait]
pub trait CandidateSource: Send + Sync {
    async fn candidates(&self, goal: &str, k: usize)
        -> Result<Vec<CapabilityDescriptor>, CapError>;
}

/// P2 — classify a goal into a (learned/semantic) [`GoalClass`] (spec R2.2).
#[async_trait]
pub trait GoalClassifier: Send + Sync {
    async fn classify(&self, goal: &str) -> GoalClass;
}

/// P2 — estimate the [`CostVector`] of a candidate/strategy (spec R15),
/// calibrated from history (spec R30.1).
#[async_trait]
pub trait CostModel: Send + Sync {
    async fn estimate(&self, descriptor: &CapabilityDescriptor) -> CostVector;
    /// Feed a measured actual back for calibration (spec R30.1).
    async fn observe(&self, provider_id: &str, capability_id: &str, actual: &CostVector);
}

/// P2 — generate multiple candidate strategies for a goal (spec R2.4).
#[async_trait]
pub trait StrategyGenerator: Send + Sync {
    async fn strategies(&self, goal: &str, class: &GoalClass) -> Vec<Strategy>;
}

/// P2 — the engineering brain: turn a goal into a [`SolutionPlan`] via the
/// tiered, budgeted, iterative pipeline (spec R2). Emits a reasoning trace.
#[async_trait]
pub trait CapabilityReasoner: Send + Sync {
    async fn reason(&self, goal: &str, ctx: &RequestContext) -> Result<SolutionPlan, CapError>;
}

/// P2 — neutral, schema-grounded, constrained argument generation (spec R3.4).
/// Relocated out of `openclaw::arg_gen`.
#[async_trait]
pub trait ArgumentGenerator: Send + Sync {
    async fn generate(
        &self,
        descriptor: &CapabilityDescriptor,
        goal: &str,
    ) -> Result<serde_json::Value, CapError>;
}

/// P3 — compose candidates into a saga-structured [`SolutionPlan`] and emit it
/// into the existing HTN runtime (spec R4). Does NOT execute directly.
#[async_trait]
pub trait CapabilityPlanner: Send + Sync {
    async fn compose(
        &self,
        goal: &str,
        class: &GoalClass,
        candidates: &[ScoredCandidate],
    ) -> Result<SolutionPlan, CapError>;
}

/// P4 — the complete capability lifecycle (spec R3/R5): transactional
/// acquire→verify→sandbox→smoke→activate, upgrade/replace, rollback, retire.
#[async_trait]
pub trait LifecycleManager: Send + Sync {
    async fn acquire_verified(&self, goal: &str) -> Result<CapabilityDescriptor, CapError>;
    async fn smoke_test(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError>;
    async fn upgrade(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError>;
    async fn rollback(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError>;
    async fn retire(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError>;
    /// Reverse a retirement (spec R19.2): re-enable an archived capability.
    async fn recover(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError>;
    async fn delete(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError>;
}

/// P8 — benchmark candidates on golden/synthetic inputs (proxy scores, spec R18).
#[async_trait]
pub trait BenchmarkHarness: Send + Sync {
    async fn benchmark(
        &self,
        provider_id: &str,
        capability_id: &str,
    ) -> Result<CostVector, CapError>;
}

/// P8 — health/benchmark-driven self-improvement, gated + reversible (spec R6).
#[async_trait]
pub trait EvolutionEngine: Send + Sync {
    /// Propose (not apply) capability migrations/replacements.
    async fn propose(&self) -> Result<Vec<DecisionRecord>, CapError>;
}

/// P4/P8 — reversible retirement of unused/deprecated capabilities (spec R19).
#[async_trait]
pub trait RetirementManager: Send + Sync {
    async fn archive(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError>;
    async fn recover(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_policy_default_is_versioned_and_sane() {
        let p = ReasoningPolicy::default();
        assert_eq!(p.version, REASONING_POLICY_VERSION);
        assert!(p.fast_path_confidence > p.native_sufficiency_confidence);
        assert!(p.native_sufficiency_confidence > p.min_action_confidence);
        assert!(p.max_rounds >= 1 && p.max_candidates >= 1 && p.budget_ms >= 1);
    }

    #[test]
    fn goal_class_open_vocabulary_roundtrips() {
        let c = GoalClass::Other("bespoke".into());
        let j = serde_json::to_string(&c).unwrap();
        let back: GoalClass = serde_json::from_str(&j).unwrap();
        assert_eq!(c, back);
    }
}
