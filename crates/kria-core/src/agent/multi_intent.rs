//! Batch 3 — Multi-Intent Decomposition (Rule + Templates + Typed Ontology)
//!
//! This module decomposes complex operational requests into bounded, typed
//! intents. It does NOT execute actions and does NOT perform environment I/O.
//! LLM assistance is optional and bounded behind a strict JSON schema.

use std::sync::Arc;

use async_trait::async_trait;

use crate::agent::intent_compiler::{ContentClass, GuiTaskSpec, IntentCompiler, TargetRef, Verb};
use crate::agent::intent_compiler_rule::RuleIntentCompiler;
use crate::agent::opgraph::{
    ConfirmationPolicy, DependencyType, IntentNode, OpEdge, OpEdgeKind, OpGraph, OpNode,
    OpNodeKind, OpNodeMetadata, WorkflowDomain,
};
use crate::agent::turn_gate::IntentEnvelope;
use crate::llm::{ChatMessage, LlmBackend};
use crate::safety::RiskLevel;

/// Maximum clauses to consider from a single user input.
pub const MAX_INTENT_CLAUSES: usize = 6;

/// Typed intent categories for operational workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentCategory {
    Coding,
    Debugging,
    Browser,
    Deployment,
    Filesystem,
    JiraDevops,
    VmContainer,
    Communication,
    Research,
    Recovery,
    SystemOperations,
    Other,
}

/// Structured GUI intent (optional) used for GoalTree compilation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiIntent {
    pub verb: Verb,
    pub targets: Vec<TargetRef>,
    pub content: Option<ContentClass>,
}

/// A single intent clause derived from user text.
#[derive(Debug, Clone)]
pub struct IntentClause {
    pub summary: String,
    pub category: IntentCategory,
    pub dependency: DependencyType,
    pub domain: WorkflowDomain,
    pub gui_intent: Option<GuiIntent>,
}

/// Decomposition quality classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecompositionQuality {
    SingleIntent,
    RuleBased,
    TemplateMatched,
    LlmAssisted,
    NeedsClarification,
}

/// Decomposition output.
#[derive(Debug, Clone)]
pub struct DecompositionResult {
    pub opgraph: OpGraph,
    pub clauses: Vec<IntentClause>,
    pub quality: DecompositionQuality,
    pub warnings: Vec<String>,
}

/// Multi-intent decomposer contract.
#[async_trait]
pub trait MultiIntentDecomposer: Send + Sync {
    async fn decompose(&self, user_text: &str, intent: &IntentEnvelope) -> DecompositionResult;
}

/// Rule-based + template-driven decomposer.
pub struct RuleBasedMultiIntentDecomposer {
    compiler: RuleIntentCompiler,
}

impl RuleBasedMultiIntentDecomposer {
    pub fn new() -> Self {
        Self {
            compiler: RuleIntentCompiler,
        }
    }
}

impl Default for RuleBasedMultiIntentDecomposer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MultiIntentDecomposer for RuleBasedMultiIntentDecomposer {
    async fn decompose(&self, user_text: &str, intent: &IntentEnvelope) -> DecompositionResult {
        let trimmed = user_text.trim();
        let mut warnings = Vec::new();
        let mut clauses = split_clauses(trimmed);

        if clauses.len() <= 1 {
            let opgraph = build_single_intent_graph(trimmed);
            return DecompositionResult {
                opgraph,
                clauses: Vec::new(),
                quality: DecompositionQuality::SingleIntent,
                warnings,
            };
        }

        if clauses.len() > MAX_INTENT_CLAUSES {
            clauses.truncate(MAX_INTENT_CLAUSES);
            warnings.push(format!(
                "Intent list truncated to {} clauses",
                MAX_INTENT_CLAUSES
            ));
        }

        if let Some(template) = WorkflowTemplateRegistry::match_template(trimmed) {
            let (opgraph, clauses) = template.to_opgraph(trimmed);
            return DecompositionResult {
                opgraph,
                clauses,
                quality: DecompositionQuality::TemplateMatched,
                warnings,
            };
        }

        let mut intent_clauses = Vec::new();
        for clause in &clauses {
            let category = IntentOntology::classify(clause);
            let domain = IntentOntology::to_domain(category);
            let dependency = DependencyType::Hard;
            let gui_intent = extract_gui_intent(&self.compiler, clause, intent).await;
            intent_clauses.push(IntentClause {
                summary: clause.clone(),
                category,
                dependency,
                domain,
                gui_intent,
            });
        }

        let opgraph = build_intent_graph(trimmed, &intent_clauses);
        DecompositionResult {
            opgraph,
            clauses: intent_clauses,
            quality: DecompositionQuality::RuleBased,
            warnings,
        }
    }
}

/// Optional LLM-assisted decomposer (bounded JSON schema).
pub struct LlmMultiIntentDecomposer {
    backend: Arc<dyn LlmBackend>,
}

impl LlmMultiIntentDecomposer {
    pub fn new(backend: Arc<dyn LlmBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl MultiIntentDecomposer for LlmMultiIntentDecomposer {
    async fn decompose(&self, user_text: &str, intent: &IntentEnvelope) -> DecompositionResult {
        let system = ChatMessage {
            role: "system".into(),
            content: LLM_SYSTEM_PROMPT.to_string(),
            name: None,
            images: None,
        };
        let user = ChatMessage {
            role: "user".into(),
            content: user_text.to_string(),
            name: None,
            images: None,
        };
        let messages = vec![system, user];
        let schema = llm_schema();
        let response = self
            .backend
            .chat_with_grammar(&messages, schema, 0.1, 512)
            .await;

        match response {
            Ok(res) => {
                let parsed = serde_json::from_str::<LlmIntentOutput>(&res.content);
                if let Ok(output) = parsed {
                    let clauses = output
                        .intents
                        .into_iter()
                        .map(|item| IntentClause {
                            summary: item.summary,
                            category: IntentOntology::from_label(&item.category),
                            dependency: IntentOntology::dependency_from_label(&item.dependency),
                            domain: IntentOntology::to_domain(IntentOntology::from_label(
                                &item.category,
                            )),
                            gui_intent: None,
                        })
                        .collect::<Vec<_>>();
                    let opgraph = build_intent_graph(user_text, &clauses);
                    return DecompositionResult {
                        opgraph,
                        clauses,
                        quality: DecompositionQuality::LlmAssisted,
                        warnings: Vec::new(),
                    };
                }
            }
            Err(e) => {
                tracing::debug!(
                    target: "multi_intent",
                    error = %e,
                    "LLM multi-intent decomposition failed; falling back to rule-based"
                );
            }
        }

        RuleBasedMultiIntentDecomposer::new()
            .decompose(user_text, intent)
            .await
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Intent ontology
// ──────────────────────────────────────────────────────────────────────────────

struct IntentOntology;

impl IntentOntology {
    fn classify(text: &str) -> IntentCategory {
        let lower = text.to_lowercase();
        if contains_any(&lower, &["debug", "fix", "bug", "error", "stack trace"]) {
            return IntentCategory::Debugging;
        }
        if contains_any(&lower, &["test", "run tests", "cargo test", "pytest"]) {
            return IntentCategory::Coding;
        }
        if contains_any(&lower, &["deploy", "release", "rollout", "restart service"]) {
            return IntentCategory::Deployment;
        }
        if contains_any(
            &lower,
            &["browser", "navigate", "open url", "website", "search"],
        ) {
            return IntentCategory::Browser;
        }
        if contains_any(
            &lower,
            &["file", "folder", "directory", "save", "copy", "move"],
        ) {
            return IntentCategory::Filesystem;
        }
        if contains_any(&lower, &["jira", "ticket", "issue", "pull request", "pr"]) {
            return IntentCategory::JiraDevops;
        }
        if contains_any(&lower, &["docker", "container", "vm", "k8s", "kubectl"]) {
            return IntentCategory::VmContainer;
        }
        if contains_any(&lower, &["email", "slack", "message", "notify"]) {
            return IntentCategory::Communication;
        }
        if contains_any(&lower, &["research", "investigate", "look up", "find out"]) {
            return IntentCategory::Research;
        }
        if contains_any(&lower, &["rollback", "recover", "restore", "retry"]) {
            return IntentCategory::Recovery;
        }
        if contains_any(&lower, &["install", "configure", "system", "update"]) {
            return IntentCategory::SystemOperations;
        }
        if contains_any(
            &lower,
            &["code", "implement", "refactor", "commit", "build"],
        ) {
            return IntentCategory::Coding;
        }
        IntentCategory::Other
    }

    fn to_domain(category: IntentCategory) -> WorkflowDomain {
        match category {
            IntentCategory::Coding => WorkflowDomain::Coding,
            IntentCategory::Debugging => WorkflowDomain::Debugging,
            IntentCategory::Browser => WorkflowDomain::Browser,
            IntentCategory::Deployment => WorkflowDomain::Deployment,
            IntentCategory::Filesystem => WorkflowDomain::Filesystem,
            IntentCategory::JiraDevops => WorkflowDomain::JiraDevops,
            IntentCategory::VmContainer => WorkflowDomain::VmContainer,
            IntentCategory::Communication => WorkflowDomain::Communication,
            IntentCategory::Research => WorkflowDomain::Research,
            IntentCategory::Recovery => WorkflowDomain::Recovery,
            IntentCategory::SystemOperations => WorkflowDomain::SystemOperations,
            IntentCategory::Other => WorkflowDomain::Unknown,
        }
    }

    fn from_label(label: &str) -> IntentCategory {
        match label {
            "coding" => IntentCategory::Coding,
            "debugging" => IntentCategory::Debugging,
            "browser" => IntentCategory::Browser,
            "deployment" => IntentCategory::Deployment,
            "filesystem" => IntentCategory::Filesystem,
            "jira_devops" => IntentCategory::JiraDevops,
            "vm_container" => IntentCategory::VmContainer,
            "communication" => IntentCategory::Communication,
            "research" => IntentCategory::Research,
            "recovery" => IntentCategory::Recovery,
            "system_operations" => IntentCategory::SystemOperations,
            _ => IntentCategory::Other,
        }
    }

    fn dependency_from_label(label: &str) -> DependencyType {
        match label {
            "soft" => DependencyType::Soft,
            "recoverable" => DependencyType::Recoverable,
            "optional" => DependencyType::Optional,
            _ => DependencyType::Hard,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Workflow templates
// ──────────────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
struct WorkflowTemplate {
    id: &'static str,
    keywords: &'static [&'static str],
    phases: &'static [TemplatePhase],
}

#[derive(Clone)]
struct TemplatePhase {
    label: &'static str,
    category: IntentCategory,
}

struct WorkflowTemplateRegistry;

impl WorkflowTemplateRegistry {
    fn match_template(text: &str) -> Option<&'static WorkflowTemplate> {
        let lower = text.to_lowercase();
        for template in TEMPLATES {
            if template.keywords.iter().all(|kw| lower.contains(kw)) {
                return Some(template);
            }
        }
        None
    }
}

impl WorkflowTemplate {
    fn to_opgraph(&self, user_text: &str) -> (OpGraph, Vec<IntentClause>) {
        let mut graph = OpGraph::new(
            format!("opg-{}", uuid::Uuid::new_v4()),
            user_text.to_string(),
        );
        let mut clauses = Vec::new();

        for (idx, phase) in self.phases.iter().enumerate() {
            let summary = phase.label.to_string();
            let category = phase.category;
            let domain = IntentOntology::to_domain(category);
            let clause = IntentClause {
                summary: summary.clone(),
                category,
                dependency: DependencyType::Hard,
                domain,
                gui_intent: None,
            };
            clauses.push(clause.clone());
            graph.nodes.push(OpNode {
                id: format!("intent_{}", idx + 1),
                label: summary,
                kind: OpNodeKind::Intent(IntentNode {
                    summary: clause.summary.clone(),
                    dependency: clause.dependency,
                    gui_intent: clause.gui_intent.clone(),
                }),
                metadata: OpNodeMetadata {
                    risk: RiskLevel::Yellow,
                    confirmation: ConfirmationPolicy::Notice,
                    workflow_domain: domain,
                    ..OpNodeMetadata::default()
                },
            });
            if idx > 0 {
                graph.edges.push(OpEdge {
                    from: format!("intent_{}", idx),
                    to: format!("intent_{}", idx + 1),
                    kind: OpEdgeKind::DependsOn,
                    dependency: DependencyType::Hard,
                });
            }
        }

        (graph, clauses)
    }
}

static TEMPLATES: &[WorkflowTemplate] = &[
    WorkflowTemplate {
        id: "coding_fix_test_commit",
        keywords: &["fix", "test", "commit"],
        phases: &[
            TemplatePhase {
                label: "Fix code changes",
                category: IntentCategory::Coding,
            },
            TemplatePhase {
                label: "Run tests",
                category: IntentCategory::Coding,
            },
            TemplatePhase {
                label: "Commit changes",
                category: IntentCategory::Coding,
            },
        ],
    },
    WorkflowTemplate {
        id: "deploy_restart_verify",
        keywords: &["deploy", "restart", "verify"],
        phases: &[
            TemplatePhase {
                label: "Deploy changes",
                category: IntentCategory::Deployment,
            },
            TemplatePhase {
                label: "Restart service",
                category: IntentCategory::Deployment,
            },
            TemplatePhase {
                label: "Verify deployment",
                category: IntentCategory::Recovery,
            },
        ],
    },
];

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

async fn extract_gui_intent(
    compiler: &RuleIntentCompiler,
    clause: &str,
    intent: &IntentEnvelope,
) -> Option<GuiIntent> {
    let spec = compiler.compile(clause, intent).await.ok()?;
    if matches!(spec.primary_verb, Verb::Other(_)) || !spec.ambiguities.is_empty() {
        return None;
    }
    Some(GuiIntent::from_spec(&spec))
}

impl GuiIntent {
    fn from_spec(spec: &GuiTaskSpec) -> Self {
        Self {
            verb: spec.primary_verb.clone(),
            targets: spec.targets.clone(),
            content: spec.content.clone(),
        }
    }
}

fn build_intent_graph(user_text: &str, clauses: &[IntentClause]) -> OpGraph {
    let mut graph = OpGraph::new(
        format!("opg-{}", uuid::Uuid::new_v4()),
        user_text.to_string(),
    );

    for (idx, clause) in clauses.iter().enumerate() {
        graph.nodes.push(OpNode {
            id: format!("intent_{}", idx + 1),
            label: clause.summary.clone(),
            kind: OpNodeKind::Intent(IntentNode {
                summary: clause.summary.clone(),
                dependency: clause.dependency,
                gui_intent: clause.gui_intent.clone(),
            }),
            metadata: OpNodeMetadata {
                risk: RiskLevel::Yellow,
                confirmation: ConfirmationPolicy::Notice,
                workflow_domain: clause.domain,
                ..OpNodeMetadata::default()
            },
        });
        if idx > 0 {
            graph.edges.push(OpEdge {
                from: format!("intent_{}", idx),
                to: format!("intent_{}", idx + 1),
                kind: OpEdgeKind::DependsOn,
                dependency: clause.dependency,
            });
        }
    }

    graph
}

fn build_single_intent_graph(user_text: &str) -> OpGraph {
    let mut graph = OpGraph::new(
        format!("opg-{}", uuid::Uuid::new_v4()),
        user_text.to_string(),
    );
    graph.nodes.push(OpNode {
        id: "intent_1".to_string(),
        label: user_text.to_string(),
        kind: OpNodeKind::Intent(IntentNode {
            summary: user_text.to_string(),
            dependency: DependencyType::Hard,
            gui_intent: None,
        }),
        metadata: OpNodeMetadata {
            risk: RiskLevel::Green,
            confirmation: ConfirmationPolicy::None,
            workflow_domain: WorkflowDomain::Unknown,
            ..OpNodeMetadata::default()
        },
    });
    graph
}

fn split_clauses(text: &str) -> Vec<String> {
    let mut clauses = Vec::new();
    let mut buffer = String::new();

    let mut i = 0;
    let lower_chars: Vec<char> = text.to_lowercase().chars().collect();
    let orig_chars: Vec<char> = text.chars().collect();
    while i < lower_chars.len() {
        let rest: String = lower_chars[i..].iter().collect();
        if rest.starts_with(";") {
            if !buffer.trim().is_empty() {
                clauses.push(buffer.trim().to_string());
            }
            buffer.clear();
            i += 1;
            continue;
        }

        let connectors = [
            (" then ", 6),
            (" and ", 5),
            (" after ", 7),
            (" before ", 8),
            (" next ", 6),
        ];
        let mut matched = false;
        for (conn, skip) in connectors {
            if rest.starts_with(conn) {
                if !buffer.trim().is_empty() {
                    clauses.push(buffer.trim().to_string());
                }
                buffer.clear();
                i += skip;
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }

        buffer.push(orig_chars[i]);
        i += 1;
    }

    if !buffer.trim().is_empty() {
        clauses.push(buffer.trim().to_string());
    }

    clauses
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

// ──────────────────────────────────────────────────────────────────────────────
// LLM schema + output types
// ──────────────────────────────────────────────────────────────────────────────

const LLM_SYSTEM_PROMPT: &str = r#"You are a bounded multi-intent decomposer.
Return only JSON with the specified schema. Do not include prose.
"#;

#[derive(Debug, serde::Deserialize)]
struct LlmIntentOutput {
    intents: Vec<LlmIntentItem>,
}

#[derive(Debug, serde::Deserialize)]
struct LlmIntentItem {
    summary: String,
    category: String,
    dependency: String,
}

fn llm_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "intents": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "summary": { "type": "string" },
                        "category": {
                            "type": "string",
                            "enum": [
                                "coding","debugging","browser","deployment","filesystem",
                                "jira_devops","vm_container","communication","research",
                                "recovery","system_operations","other"
                            ]
                        },
                        "dependency": {
                            "type": "string",
                            "enum": ["hard","soft","recoverable","optional"]
                        }
                    },
                    "required": ["summary","category","dependency"]
                },
                "maxItems": MAX_INTENT_CLAUSES
            }
        },
        "required": ["intents"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::turn_gate::{ComputeClass, HazardHint, IntentSource, Modality, Operation};

    #[tokio::test]
    async fn split_and_builds_graph() {
        let decomposer = RuleBasedMultiIntentDecomposer::new();
        let intent = IntentEnvelope::new(
            Modality::Text,
            Operation::Automate,
            HazardHint::Green,
            ComputeClass::ToolOnly,
            0.9,
            IntentSource::DeterministicGuard,
        );
        let result = decomposer
            .decompose("open gedit and type hello", &intent)
            .await;
        assert!(result.opgraph.nodes.len() >= 2);
    }

    #[test]
    fn ontology_classifies_debugging() {
        let cat = IntentOntology::classify("fix the error and debug logs");
        assert_eq!(cat, IntentCategory::Debugging);
    }
}
