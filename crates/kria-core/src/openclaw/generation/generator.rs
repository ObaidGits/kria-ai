//! Skill generator abstraction — the LLM boundary for A9.
//!
//! The pipeline depends on the `SkillGenerator` trait, never on a concrete LLM. This
//! keeps the pipeline deterministic + fully testable (mock generator) and lets the
//! production `LlmSkillGenerator` swap in without pipeline changes (A9.14).

use super::designer::SkillDesign;
use super::requirements::SkillRequirement;
use async_trait::async_trait;

/// Generated implementation artifacts for a skill (A9.5).
#[derive(Debug, Clone)]
pub struct GeneratedArtifacts {
    /// Handler source code (entry file contents).
    pub handler_code: String,
    /// Unit/integration test source (runs inside the sandbox).
    pub test_code: String,
    /// Additional example invocations rendered as docs.
    pub examples_doc: String,
}

/// Errors from the generator (LLM/parse failures).
#[derive(Debug, thiserror::Error)]
pub enum GeneratorError {
    #[error("llm error: {0}")]
    Llm(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("token estimate: {0}")]
    Tokens(u64),
}

/// The generator interface (A9.2/A9.3/A9.5/A9.8). Every method reports an estimated
/// token cost so the pipeline can charge the budget.
#[async_trait]
pub trait SkillGenerator: Send + Sync {
    /// A9.2: extract a structured requirement from a raw user prompt.
    async fn extract_requirements(
        &self,
        prompt: &str,
    ) -> Result<(SkillRequirement, u64), GeneratorError>;

    /// A9.3: design a skill from a requirement.
    async fn design_skill(
        &self,
        req: &SkillRequirement,
    ) -> Result<(SkillDesign, u64), GeneratorError>;

    /// A9.5: generate production implementation artifacts for a design.
    async fn generate_code(
        &self,
        design: &SkillDesign,
    ) -> Result<(GeneratedArtifacts, u64), GeneratorError>;

    /// A9.8: repair a failing skill given the failure detail. Returns new artifacts.
    async fn repair_code(
        &self,
        design: &SkillDesign,
        current: &GeneratedArtifacts,
        failure: &str,
    ) -> Result<(GeneratedArtifacts, u64), GeneratorError>;
}
