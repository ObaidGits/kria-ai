//! RFC v2 (P1): Semantic intent normalization layer.
//!
//! This module is intentionally a *normalizer*, not a planner. It produces a
//! typed [`GuiTaskSpec`] from natural-language user input and the
//! [`IntentEnvelope`] supplied by [`TurnGate`]. It MUST NOT:
//!
//! - emit step lists,
//! - read environment state (no screen captures, no `/proc`, no window queries),
//! - call out to OmniParser,
//! - mutate any global state.
//!
//! See `docs/GUI_INTELLIGENCE_REVIEW.md` §4.2 for the contract and rationale.

use crate::agent::turn_gate::IntentEnvelope;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// High-level verb the user wants the GUI agent to perform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verb {
    Open,
    Type,
    Click,
    Run,
    Save,
    Close,
    Switch,
    Other(String),
}

/// Reference to a target object named in the user's request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetRef {
    App(String),
    File(PathBuf),
    Url(String),
    Element(String),
}

/// Classification of the content the user wants placed somewhere.
///
/// Distinguishes `Literal(text)` ("type 'hello world'") from `Generated`
/// ("write a fibonacci program"). The executor uses this to choose between
/// `TextPresent` verification (literal) and `CompletionFlag` verification
/// (generated, to avoid false-positive re-execution on perceptual diff).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentClass {
    Literal(String),
    Generated {
        hint: String,
        language: Option<String>,
    },
}

/// A precondition the user has *implicitly or explicitly declared* in the
/// request. The Grounder checks these; the Planner uses them to seed
/// prerequisite-sense Goal Tree leaves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrereqHint {
    AppOpen(String),
    FileExists(PathBuf),
    Focused(TargetRef),
}

/// The user's stated or strongly-implied success criterion. The Verifier
/// turns these into [`crate::agent::execution_verifier::Verifiability`] leaves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuccessHint {
    TextInFile { path: PathBuf, substring: String },
    ProcessExited(u32),
    WindowVisible(String),
    UserConfirmed,
}

/// Surfaced ambiguity. Drives the clarify path; the compiler refuses to guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ambiguity {
    AppNotSpecified,
    FileNotSpecified,
    MultipleTargetsPossible,
    ContentScopeUnclear,
}

/// The complete typed specification produced by the compiler.
#[derive(Debug, Clone)]
pub struct GuiTaskSpec {
    pub primary_verb: Verb,
    pub targets: Vec<TargetRef>,
    pub content: Option<ContentClass>,
    pub declared_preconditions: Vec<PrereqHint>,
    pub declared_success_criteria: Vec<SuccessHint>,
    pub ambiguities: Vec<Ambiguity>,
}

/// A clarification request, raised when ambiguity prevents safe progress.
#[derive(Debug, Clone)]
pub struct ClarifyRequest {
    pub question: String,
    pub options: Vec<String>,
}

/// The compiler contract.
#[async_trait::async_trait]
pub trait IntentCompiler: Send + Sync {
    /// Compile user text into a typed task spec. Returns `Err(ClarifyRequest)`
    /// when any blocking [`Ambiguity`] is present; the caller MUST forward
    /// the clarification to the user instead of attempting execution.
    async fn compile(
        &self,
        user_text: &str,
        intent: &IntentEnvelope,
    ) -> Result<GuiTaskSpec, ClarifyRequest>;
}

/// Default no-op compiler — placeholder until P1 implementation lands.
///
/// Returns a `GuiTaskSpec` with `Verb::Other("noop")` so call sites can wire
/// the trait without changing behaviour. P1 replaces this with a real
/// implementation.
pub struct NoopIntentCompiler;

#[async_trait::async_trait]
impl IntentCompiler for NoopIntentCompiler {
    async fn compile(
        &self,
        _user_text: &str,
        _intent: &IntentEnvelope,
    ) -> Result<GuiTaskSpec, ClarifyRequest> {
        Ok(GuiTaskSpec {
            primary_verb: Verb::Other("noop".to_string()),
            targets: Vec::new(),
            content: None,
            declared_preconditions: Vec::new(),
            declared_success_criteria: Vec::new(),
            ambiguities: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::intent_compiler::IntentCompiler;
    use crate::agent::turn_gate::{ComputeClass, HazardHint, IntentSource, Modality, Operation};

    #[tokio::test]
    async fn noop_compiler_never_errors() {
        let compiler = NoopIntentCompiler;
        let intent = IntentEnvelope::new(
            Modality::Text,
            Operation::Automate,
            HazardHint::Green,
            ComputeClass::ToolOnly,
            0.9,
            IntentSource::FastEmbedSemanticRouter,
        );
        let spec = compiler
            .compile("anything", &intent)
            .await
            .expect("noop never errs");
        assert!(spec.targets.is_empty());
        assert!(spec.ambiguities.is_empty());
    }
}
