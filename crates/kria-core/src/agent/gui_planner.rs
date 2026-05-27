//! GuiPlanner — single planner authority.
//!
//! P2: Planners receive `&OperationalFacts` from the EnvironmentGrounder.
//! Facts are advisory — planners MAY optimize step ordering using facts
//! but MUST NOT remove prerequisite verification steps.
//!
//! Replaces the parallel routing stacks (rule-based planner vs LLM HTN planner)
//! with a unified trait.  The planner receives a typed `GuiTaskSpec` from the
//! `IntentCompiler` and produces a `GuiWorkflow`.
//!
//! Design constraints (per RFC 007 / RFC 008):
//! * Planner MUST NEVER execute.
//! * Planner MUST return `GuiWorkflow` — never raw tool calls.
//! * Rule-based path is <5 ms, LLM path is bounded by timeout + max tokens.
//! * No hidden recursion, no replanning inside the planner.

use crate::agent::environment_grounder::OperationalFacts;
use crate::agent::htn_executor::{GuiWorkflow, SafeAbortStep};
use crate::agent::intent_compiler::GuiTaskSpec;
use std::sync::Arc;

/// Unified planner trait.
///
/// All GUI planning flows through this trait.  Implementations may be
/// rule-based, LLM-based, or hybrid — the caller does not care.
#[async_trait::async_trait]
pub trait GuiPlanner: Send + Sync {
    /// Produce a workflow from a semantically-normalised task spec.
    ///
    /// # Errors
    /// Returns `PlannerError` when the intent cannot be mapped to a safe
    /// workflow (e.g. unsupported verb, missing parameters, or LLM failure).
    async fn plan(
        &self,
        spec: &GuiTaskSpec,
        facts: &OperationalFacts,
    ) -> Result<GuiWorkflow, PlannerError>;
}

/// Planner failure modes — always explicit, never silent.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PlannerError {
    #[error("Unsupported intent verb: {0}")]
    UnsupportedVerb(String),
    #[error("Missing required parameter: {0}")]
    MissingParameter(String),
    #[error("LLM planning failed: {0}")]
    LlmFailure(String),
    #[error("Safety policy blocked: {0}")]
    SafetyBlocked(String),
    #[error("Ambiguous target: {0}")]
    AmbiguousTarget(String),
}

/// Fast rule-based planner for deterministic GUI workflows.
///
/// Handles the common verbs (`Open`, `Type`, `Click`, `Close`, `Switch`)
/// by mapping them directly to a `GuiWorkflow` with appropriate sub-goals
/// and verification hints.
pub struct RuleBasedPlanner;

#[async_trait::async_trait]
impl GuiPlanner for RuleBasedPlanner {
    async fn plan(
        &self,
        spec: &GuiTaskSpec,
        _facts: &OperationalFacts,
    ) -> Result<GuiWorkflow, PlannerError> {
        use crate::agent::htn_executor::{GuiWorkflow, SubGoal, VerificationType};
        use crate::agent::intent_compiler::{ContentClass, TargetRef, Verb};

        let mut sub_goals = Vec::new();

        match spec.primary_verb {
            Verb::Open => {
                let app = spec
                    .targets
                    .iter()
                    .find_map(|t| match t {
                        TargetRef::App(a) => Some(a.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| PlannerError::MissingParameter("target app".into()))?;

                // FIX: Use ProcessLaunched instead of WindowState.
                // WindowState requires xdotool IPC which always fails on Wayland
                // (WINDOW_ID_FAILED). ProcessLaunched polls /proc directly and
                // works on both X11 and Wayland without any IPC dependency.
                let binary = crate::agent::gui_substrate_planner::app_alias_to_binary_pub(&app);
                sub_goals.push(SubGoal {
                    step: 1,
                    action: "open_application".into(),
                    params: serde_json::json!({"name": app}),
                    verify: VerificationType::ProcessLaunched {
                        binary,
                        max_wait_ms: 6000,
                    },
                    timeout_ms: Some(8000),
                });
            }
            Verb::Type => {
                let text = match spec.content.as_ref() {
                    Some(ContentClass::Literal(t)) => t.clone(),
                    Some(ContentClass::Generated { hint, .. }) => hint.clone(),
                    _ => return Err(PlannerError::MissingParameter("text to type".into())),
                };

                sub_goals.push(SubGoal {
                    step: 1,
                    action: "type_text".into(),
                    params: serde_json::json!({"text": text}),
                    verify: VerificationType::TextPresent {
                        text: text.clone(),
                        case_insensitive: false,
                    },
                    timeout_ms: Some(2000),
                });
            }
            Verb::Click => {
                let element = spec
                    .targets
                    .iter()
                    .find_map(|t| match t {
                        TargetRef::Element(e) => Some(e.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| PlannerError::MissingParameter("target element".into()))?;

                sub_goals.push(SubGoal {
                    step: 1,
                    action: "click_element".into(),
                    params: serde_json::json!({"element_id": element, "button": "left"}),
                    verify: VerificationType::ScreenChanged {
                        element_id: None,
                        threshold: 0.90,
                    },
                    timeout_ms: Some(2000),
                });
            }
            Verb::Close => {
                let app = spec
                    .targets
                    .iter()
                    .find_map(|t| match t {
                        TargetRef::App(a) => Some(a.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| PlannerError::MissingParameter("target app".into()))?;

                sub_goals.push(SubGoal {
                    step: 1,
                    action: "close_application".into(),
                    params: serde_json::json!({"name": app}),
                    verify: VerificationType::None,
                    timeout_ms: Some(2000),
                });
            }
            Verb::Switch => {
                let app = spec
                    .targets
                    .iter()
                    .find_map(|t| match t {
                        TargetRef::App(a) => Some(a.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| PlannerError::MissingParameter("target app".into()))?;

                // FIX: Use ProcessLaunched for Switch too — WindowState requires
                // xdotool IPC which fails on Wayland. ProcessLaunched verifies
                // the process is running, which is the meaningful check here.
                let binary = crate::agent::gui_substrate_planner::app_alias_to_binary_pub(&app);
                sub_goals.push(SubGoal {
                    step: 1,
                    action: "switch_to_window".into(),
                    params: serde_json::json!({"name": app}),
                    verify: VerificationType::ProcessLaunched {
                        binary,
                        max_wait_ms: 3000,
                    },
                    timeout_ms: Some(5000),
                });
            }
            Verb::Run => {
                let cmd = spec
                    .targets
                    .iter()
                    .find_map(|t| match t {
                        TargetRef::App(a) => Some(a.clone()),
                        _ => None,
                    })
                    .or_else(|| {
                        spec.content.as_ref().and_then(|c| match c {
                            ContentClass::Literal(t) => Some(t.clone()),
                            _ => None,
                        })
                    })
                    .ok_or_else(|| PlannerError::MissingParameter("command to run".into()))?;

                sub_goals.push(SubGoal {
                    step: 1,
                    action: "run_command".into(),
                    params: serde_json::json!({"command": cmd}),
                    verify: VerificationType::None,
                    timeout_ms: Some(5000),
                });
            }
            Verb::Save => {
                sub_goals.push(SubGoal {
                    step: 1,
                    action: "press_shortcut".into(),
                    params: serde_json::json!({"keys": ["Ctrl+S"]}),
                    verify: VerificationType::None,
                    timeout_ms: Some(2000),
                });
            }
            Verb::Other(ref raw) => {
                return Err(PlannerError::UnsupportedVerb(raw.clone()));
            }
        }

        // Safe abort: always include Escape shortcut per RFC 007
        let safe_abort_steps = vec![SafeAbortStep {
            action: "press_shortcut".into(),
            params: serde_json::json!({"keys": ["Escape"]}),
        }];

        Ok(GuiWorkflow {
            task_id: format!("rule-{}", uuid::Uuid::new_v4()),
            sub_goals,
            safe_abort_steps,
            max_duration_sec: 60,
        })
    }
}

/// Fallback planner that delegates to the existing LLM HTN planner.
///
/// This is a thin wrapper around `htn_integration::plan_gui_workflow_via_llm`
/// so the LLM path still exists as a single trait implementation rather than
/// a parallel routing stack.
pub struct LlmHtnPlanner {
    backend: Arc<dyn crate::llm::LlmBackend>,
}

impl LlmHtnPlanner {
    pub fn new(backend: Arc<dyn crate::llm::LlmBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait::async_trait]
impl GuiPlanner for LlmHtnPlanner {
    async fn plan(
        &self,
        spec: &GuiTaskSpec,
        _facts: &OperationalFacts,
    ) -> Result<GuiWorkflow, PlannerError> {
        // Convert GuiTaskSpec to a natural-language description for the LLM
        let description = format!("{:#?}", spec);
        match crate::agent::htn_integration::plan_gui_workflow_via_llm(
            self.backend.as_ref(),
            "llm-plan",
            &description,
        )
        .await
        {
            Ok(workflow) => Ok(workflow),
            Err(e) => Err(PlannerError::LlmFailure(e.to_string())),
        }
    }
}

/// Composite planner: tries rule-based first, falls back to LLM on
/// `UnsupportedVerb` or `MissingParameter`.
///
/// This preserves the fast-path / slow-path split without exposing two
/// separate routing stacks to the caller.
pub struct SimplePlanner {
    rule: RuleBasedPlanner,
    llm: LlmHtnPlanner,
}

impl SimplePlanner {
    pub fn new(llm_backend: Arc<dyn crate::llm::LlmBackend>) -> Self {
        Self {
            rule: RuleBasedPlanner,
            llm: LlmHtnPlanner::new(llm_backend),
        }
    }
}

#[async_trait::async_trait]
impl GuiPlanner for SimplePlanner {
    async fn plan(
        &self,
        spec: &GuiTaskSpec,
        facts: &OperationalFacts,
    ) -> Result<GuiWorkflow, PlannerError> {
        match self.rule.plan(spec, facts).await {
            Ok(workflow) => Ok(workflow),
            Err(PlannerError::UnsupportedVerb(_) | PlannerError::MissingParameter(_)) => {
                tracing::info!(verb = ?spec.primary_verb, "Rule planner declined, falling back to LLM planner");
                self.llm.plan(spec, facts).await
            }
            Err(other) => Err(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::environment_grounder::{GroundingCapabilities, OperationalFacts};
    use crate::agent::intent_compiler::{ContentClass, GuiTaskSpec, TargetRef, Verb};

    #[tokio::test]
    async fn rule_planner_open_gedit() {
        let spec = GuiTaskSpec {
            primary_verb: Verb::Open,
            targets: vec![TargetRef::App("gedit".into())],
            content: None,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        };
        let planner = RuleBasedPlanner;
        let facts = OperationalFacts::empty(GroundingCapabilities::none());
        let workflow = planner.plan(&spec, &facts).await.unwrap();
        assert_eq!(workflow.sub_goals.len(), 1);
        assert_eq!(workflow.sub_goals[0].action, "open_application");
    }

    #[tokio::test]
    async fn rule_planner_unsupported_verb() {
        let spec = GuiTaskSpec {
            primary_verb: Verb::Other("dance".into()),
            targets: vec![],
            content: None,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        };
        let planner = RuleBasedPlanner;
        let facts = OperationalFacts::empty(GroundingCapabilities::none());
        let err = planner.plan(&spec, &facts).await.unwrap_err();
        assert!(matches!(err, PlannerError::UnsupportedVerb(_)));
    }

    #[tokio::test]
    async fn simple_planner_rule_fast_path() {
        use crate::llm::{ChatMessage, LlmBackend, LlmResponse};

        struct DummyBackend;
        #[async_trait::async_trait]
        impl LlmBackend for DummyBackend {
            fn model_label(&self) -> &str {
                "dummy"
            }
            fn capabilities(&self) -> &[String] {
                &[]
            }
            fn is_configured(&self) -> bool {
                false
            }
            fn tokenizer_base_url(&self) -> String {
                String::new()
            }
            async fn chat(
                &self,
                _msgs: &[ChatMessage],
                _tools: Option<&[crate::llm::ToolSchema]>,
                _temp: f32,
                _max: u32,
            ) -> anyhow::Result<LlmResponse> {
                anyhow::bail!("dummy")
            }
            async fn chat_stream(
                &self,
                _msgs: &[ChatMessage],
                _tools: Option<&[crate::llm::ToolSchema]>,
                _temp: f32,
                _max: u32,
            ) -> anyhow::Result<std::pin::Pin<Box<dyn futures::Stream<Item = String> + Send>>>
            {
                use futures::StreamExt;
                Ok(futures::stream::empty().boxed())
            }
            async fn health_check(&self) -> bool {
                false
            }
        }

        let spec = GuiTaskSpec {
            primary_verb: Verb::Click,
            targets: vec![TargetRef::Element("save".into())],
            content: None,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        };
        let planner = SimplePlanner::new(Arc::new(DummyBackend));
        let facts = OperationalFacts::empty(GroundingCapabilities::none());
        let workflow = planner.plan(&spec, &facts).await.unwrap();
        assert_eq!(workflow.sub_goals[0].action, "click_element");
    }
}
