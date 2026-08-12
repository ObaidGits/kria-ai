//! A9 Autonomous Skill Generation System (ASGS).
//!
//! Transforms OpenClaw into a self-extending platform: understand a goal → reuse an
//! existing skill if suitable → otherwise design, generate, validate, sandbox-test,
//! repair, package, sign, install, register and execute a new skill — all through the
//! SAME frozen lifecycle as manual skills (A9.15). No parallel AI pipeline exists.
//!
//! # Single-authority map (self-audit targets)
//!
//! | Concern              | Single owner                                        |
//! |----------------------|-----------------------------------------------------|
//! | Generation pipeline  | `pipeline::GenerationPipeline`                      |
//! | Decision/similarity  | `decision::{DecisionEngine, SimilarityEngine}`      |
//! | Requirement extract  | `requirements` + `generator::SkillGenerator`        |
//! | Designer             | `designer` (+ capability inference)                 |
//! | Code generator       | `codegen::emit_bundle`                              |
//! | Validator            | `validator::SkillValidator`                         |
//! | Repair               | pipeline repair loop + `generator::repair_code`     |
//! | Quality              | `quality::QualityEvaluator`                         |
//! | Budget               | `budget::GenerationBudget`                          |
//! | Approval             | `approval::ApprovalLayer`                           |
//! | Sandbox              | `sandbox::SandboxTester`                            |
//! | Events               | `events::GenerationEventStream`                     |
//! | Packaging/install    | REUSES `bundle` + `platform` (A9.9)                 |
//!
//! Packaging, signing, verification, installation, registry, marketplace, execution are
//! all REUSED from frozen phases — never re-implemented here.

pub mod approval;
pub mod budget;
pub mod codegen;
pub mod decision;
pub mod designer;
pub mod events;
pub mod generator;
pub mod install_sink;
pub mod llm_generator;
pub mod pipeline;
pub mod quality;
pub mod requirements;
pub mod sandbox;
pub mod validator;


// ── Public ASGS API ──
pub use approval::{ApprovalDecision, ApprovalLayer, ApprovalRequirement};
pub use budget::{BudgetDimension, BudgetLimits, GenerationBudget};
pub use codegen::{emit_bundle, CodegenError};
pub use decision::{
    DecisionEngine, GenerationDecision, GenerationPolicy, SimilarityEngine, SimilarityScore,
    SkillCandidate,
};
pub use designer::{
    capabilities_requiring_approval, classify_risk, infer_capabilities, SkillDesign, SkillExample,
    HIGH_RISK_CAPABILITIES,
};
pub use events::{GenerationEvent, GenerationEventStream};
pub use generator::{GeneratedArtifacts, GeneratorError, SkillGenerator};
pub use install_sink::BundleInstallSink;
pub use llm_generator::LlmSkillGenerator;
pub use pipeline::{GenerationPipeline, InstallSink, PipelineConfig, PipelineOutcome};
pub use quality::{QualityEvaluator, QualityScore};
pub use requirements::{RequirementField, SkillRequirement};
pub use sandbox::{SandboxResult, SandboxTester, StaticSandbox};
pub use validator::{SkillValidator, ValidationIssue};
