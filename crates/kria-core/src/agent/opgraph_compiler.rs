//! Batch 3 — OpGraph → GoalTree compiler
//!
//! This compiler enforces that OpGraph remains a planning abstraction.
//! It produces an immutable GoalTree for StageExecutor execution.

use crate::agent::environment_grounder::{GroundingCapabilities, OperationalFacts};
use crate::agent::goal_tree::{
    ActionGroup, CompletionContract, GoalTree, GoalTreeValidationError, Precondition, WorkflowStage,
};
use crate::agent::opgraph::{OpGraph, OpGraphValidationError, OpNodeKind};
use crate::agent::workflow_compiler::{
    MultiVerbSpec, RuleBasedWorkflowCompiler, VerbClause, WorkflowCompiler,
};

/// Compilation errors for OpGraph.
#[derive(Debug, Clone, thiserror::Error)]
pub enum OpGraphCompileError {
    #[error("OpGraph validation failed: {0:?}")]
    InvalidGraph(Vec<OpGraphValidationError>),
    #[error("No executable action stages or GUI intent clauses found")]
    NoExecutableStages,
    #[error("GoalTree validation failed: {errors:?}")]
    GoalTreeValidation {
        errors: Vec<GoalTreeValidationError>,
    },
    #[error("WorkflowCompiler failed: {0}")]
    WorkflowCompilerError(String),
}

/// Compiler that produces GoalTree from OpGraph.
pub struct GoalTreeOpGraphCompiler;

impl GoalTreeOpGraphCompiler {
    pub fn compile(
        &self,
        graph: &OpGraph,
        facts: Option<&OperationalFacts>,
    ) -> Result<GoalTree, OpGraphCompileError> {
        let validation_errors = graph.validate();
        if !validation_errors.is_empty() {
            return Err(OpGraphCompileError::InvalidGraph(validation_errors));
        }

        // Prefer explicit ActionStage nodes if present.
        let stages = self.compile_action_stages(graph)?;
        if !stages.is_empty() {
            return self.build_goal_tree(graph, stages);
        }

        // Fallback: compile from GUI intent clauses via RuleBasedWorkflowCompiler.
        self.compile_from_gui_intents(graph, facts)
    }

    fn compile_action_stages(
        &self,
        graph: &OpGraph,
    ) -> Result<Vec<WorkflowStage>, OpGraphCompileError> {
        let ordered = graph
            .topo_order()
            .map_err(|e| OpGraphCompileError::InvalidGraph(vec![e]))?;

        let mut stages = Vec::new();
        for (index, node_id) in ordered.iter().enumerate() {
            let Some(node) = graph.nodes.iter().find(|n| &n.id == node_id) else {
                continue;
            };
            if let OpNodeKind::ActionStage(stage) = &node.kind {
                let action_group = ActionGroup {
                    actions: stage.actions.clone(),
                };
                let recovery = stage.recovery.clone();
                let workflow_stage = WorkflowStage {
                    index: index as u32,
                    label: node.label.clone(),
                    action_group,
                    checkpoint: stage.checkpoint.clone(),
                    recovery,
                    context_hints: stage.context_hints.clone(),
                    timeout_sec: stage.timeout_sec,
                    skippable: stage.skippable,
                };
                stages.push(workflow_stage);
            }
        }

        Ok(stages)
    }

    fn compile_from_gui_intents(
        &self,
        graph: &OpGraph,
        facts: Option<&OperationalFacts>,
    ) -> Result<GoalTree, OpGraphCompileError> {
        let mut clauses = Vec::new();
        for node in &graph.nodes {
            if let OpNodeKind::Intent(intent) = &node.kind {
                if let Some(gui_intent) = &intent.gui_intent {
                    clauses.push(VerbClause {
                        verb: gui_intent.verb.clone(),
                        targets: gui_intent.targets.clone(),
                        content: gui_intent.content.clone(),
                    });
                }
            }
        }

        if clauses.is_empty() {
            return Err(OpGraphCompileError::NoExecutableStages);
        }

        let spec = MultiVerbSpec {
            original_text: graph.description.clone(),
            clauses,
        };

        let empty_facts = OperationalFacts::empty(GroundingCapabilities::none());
        let facts = facts.unwrap_or(&empty_facts);
        let compiler = RuleBasedWorkflowCompiler;
        let tree = compiler
            .compile(&spec, facts)
            .map_err(|e| OpGraphCompileError::WorkflowCompilerError(e.to_string()))?;

        let errors = tree.validate();
        if !errors.is_empty() {
            return Err(OpGraphCompileError::GoalTreeValidation { errors });
        }

        Ok(tree)
    }

    fn build_goal_tree(
        &self,
        graph: &OpGraph,
        mut stages: Vec<WorkflowStage>,
    ) -> Result<GoalTree, OpGraphCompileError> {
        if stages.is_empty() {
            return Err(OpGraphCompileError::NoExecutableStages);
        }

        for (index, stage) in stages.iter_mut().enumerate() {
            stage.index = index as u32;
        }

        let tree = GoalTree {
            workflow_id: graph.graph_id.clone(),
            description: graph.description.clone(),
            stages,
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![],
            max_total_duration_sec: crate::agent::goal_tree::MAX_WORKFLOW_DURATION_SEC,
            preconditions: vec![Precondition::DisplayServerAvailable],
        };

        let errors = tree.validate();
        if !errors.is_empty() {
            return Err(OpGraphCompileError::GoalTreeValidation { errors });
        }

        Ok(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::intent_compiler::{ContentClass, TargetRef, Verb};
    use crate::agent::multi_intent::GuiIntent;
    use crate::agent::opgraph::{
        ConfirmationPolicy, IntentNode, OpNode, OpNodeKind, OpNodeMetadata,
    };
    use crate::safety::RiskLevel;

    #[test]
    fn compiles_from_gui_intents() {
        let mut graph = OpGraph::new("opg-test", "open editor and run tests");
        graph.nodes.push(OpNode {
            id: "intent_1".into(),
            label: "Open editor".into(),
            kind: OpNodeKind::Intent(IntentNode {
                summary: "Open editor".into(),
                dependency: crate::agent::opgraph::DependencyType::Hard,
                gui_intent: Some(GuiIntent {
                    verb: Verb::Open,
                    targets: vec![TargetRef::App("gedit".into())],
                    content: None,
                }),
            }),
            metadata: OpNodeMetadata {
                risk: RiskLevel::Green,
                confirmation: ConfirmationPolicy::None,
                ..OpNodeMetadata::default()
            },
        });
        graph.nodes.push(OpNode {
            id: "intent_2".into(),
            label: "Run tests".into(),
            kind: OpNodeKind::Intent(IntentNode {
                summary: "Run tests".into(),
                dependency: crate::agent::opgraph::DependencyType::Hard,
                gui_intent: Some(GuiIntent {
                    verb: Verb::Run,
                    targets: vec![TargetRef::App("cargo test".into())],
                    content: Some(ContentClass::Literal("cargo test".into())),
                }),
            }),
            metadata: OpNodeMetadata::default(),
        });

        let compiler = GoalTreeOpGraphCompiler;
        let tree = compiler.compile(&graph, None).expect("should compile");
        assert_eq!(tree.stages.len(), 2);
    }
}
