//! Phase 1 governance metadata for GUI cognition evals.
//!
//! This module is intentionally small. It adds capability mapping, cost tiers,
//! ownership, priorities, and dedup keys without changing runner behavior.

use super::types::{DisplayServerRequirement, ExpectedBehavior, GuiEvalCase};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const DEDUP_GROUP_LIMIT: usize = 25;
const DEDUP_CASE_LIMIT: usize = 20;
const COVERAGE_CASE_LIMIT: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalPriority {
    P0SafetyCritical,
    P1ReleaseCriticalRuntime,
    P2CoreGuiRegression,
    P3WorkflowCognition,
    P4ProviderModelAdvisory,
    P5ExploratoryResearch,
    P6FlakyObservational,
}

impl EvalPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::P0SafetyCritical => "p0_safety_critical",
            Self::P1ReleaseCriticalRuntime => "p1_release_critical_runtime",
            Self::P2CoreGuiRegression => "p2_core_gui_regression",
            Self::P3WorkflowCognition => "p3_workflow_cognition",
            Self::P4ProviderModelAdvisory => "p4_provider_model_advisory",
            Self::P5ExploratoryResearch => "p5_exploratory_research",
            Self::P6FlakyObservational => "p6_flaky_observational",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalCostClass {
    C0FastDeterministic,
    C1FastNoDisplay,
    C2LocalGuiSmoke,
    C3FullGuiRegression,
    C4WithLlmCognition,
    C5LongHorizonHitl,
    C6DestructiveVm,
}

impl EvalCostClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::C0FastDeterministic => "c0_fast_deterministic",
            Self::C1FastNoDisplay => "c1_fast_no_display",
            Self::C2LocalGuiSmoke => "c2_local_gui_smoke",
            Self::C3FullGuiRegression => "c3_full_gui_regression",
            Self::C4WithLlmCognition => "c4_with_llm_cognition",
            Self::C5LongHorizonHitl => "c5_long_horizon_hitl",
            Self::C6DestructiveVm => "c6_destructive_vm",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalOracleType {
    ToolTrace,
    ArtifactContent,
    ProcessState,
    BrowserState,
    GuiSemanticState,
    CompositeInvariant,
}

impl EvalOracleType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ToolTrace => "tool_trace",
            Self::ArtifactContent => "artifact_content",
            Self::ProcessState => "process_state",
            Self::BrowserState => "browser_state",
            Self::GuiSemanticState => "gui_semantic_state",
            Self::CompositeInvariant => "composite_invariant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalEnvironmentProfile {
    HostNoDisplay,
    HostGuiAny,
    HostGuiX11,
    HostGuiWayland,
    VmSnapshot,
    DisposableContainer,
}

impl EvalEnvironmentProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HostNoDisplay => "host_no_display",
            Self::HostGuiAny => "host_gui_any",
            Self::HostGuiX11 => "host_gui_x11",
            Self::HostGuiWayland => "host_gui_wayland",
            Self::VmSnapshot => "vm_snapshot",
            Self::DisposableContainer => "disposable_container",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalLifecycleState {
    Proposed,
    Experimental,
    Regression,
    ReleaseCritical,
    Quarantined,
    Deprecated,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDefinition {
    pub id: String,
    pub owner: String,
    pub risk_level: CapabilityRiskLevel,
    pub protected_failure_modes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvalGovernanceMetadata {
    pub capability_ids: Vec<String>,
    pub failure_mode_ids: Vec<String>,
    pub priority: Option<EvalPriority>,
    pub cost_class: Option<EvalCostClass>,
    pub environment_profile: Option<EvalEnvironmentProfile>,
    pub oracle_type: Option<EvalOracleType>,
    pub owner: Option<String>,
    pub lifecycle: Option<EvalLifecycleState>,
    pub cleanup_contract: Option<String>,
    pub max_runtime_ms: Option<u64>,
    pub dedup_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityCoverageEntry {
    pub capability_id: String,
    pub owner: String,
    pub risk_level: CapabilityRiskLevel,
    pub protected_failure_modes: Vec<String>,
    pub linked_eval_count: usize,
    pub linked_eval_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceBreakdown {
    pub key: String,
    pub count: usize,
    pub case_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupFinding {
    pub dedup_key: String,
    pub duplicate_case_count: usize,
    pub case_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceEntropy {
    pub total_cases: usize,
    pub duplicate_group_count: usize,
    pub duplicate_case_count: usize,
    pub duplicate_ratio: f32,
    pub missing_metadata_count: usize,
    pub ownerless_count: usize,
    pub low_oracle_strength_count: usize,
    pub entropy_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceReport {
    pub capabilities: Vec<CapabilityCoverageEntry>,
    pub cost_breakdown: Vec<GovernanceBreakdown>,
    pub priority_breakdown: Vec<GovernanceBreakdown>,
    pub duplicate_dedup_keys: Vec<DedupFinding>,
    pub entropy: GovernanceEntropy,
    pub missing_metadata_cases: Vec<String>,
}

pub fn capability_registry() -> Vec<CapabilityDefinition> {
    vec![
        capability(
            "intent.app_alias",
            "kria-gui-runtime",
            CapabilityRiskLevel::Medium,
            &["wrong_app_target", "app_resolution"],
        ),
        capability(
            "intent.language_detection",
            "kria-gui-runtime",
            CapabilityRiskLevel::Medium,
            &["wrong_artifact_language", "semantic_parsing"],
        ),
        capability(
            "intent.multi_step_gui",
            "kria-gui-runtime",
            CapabilityRiskLevel::High,
            &["workflow_drift", "partial_completion"],
        ),
        capability(
            "substrate.app_lifecycle",
            "kria-gui-runtime",
            CapabilityRiskLevel::Medium,
            &["app_not_opened", "duplicate_window"],
        ),
        capability(
            "substrate.file_write_open",
            "kria-gui-runtime",
            CapabilityRiskLevel::High,
            &["artifact_missing", "wrong_editor_target"],
        ),
        capability(
            "substrate.terminal_execution",
            "kria-gui-runtime",
            CapabilityRiskLevel::High,
            &["command_not_executed", "output_missing"],
        ),
        capability(
            "substrate.browser_cdp",
            "kria-gui-runtime",
            CapabilityRiskLevel::High,
            &["wrong_browser_state", "retrieval_leakage"],
        ),
        capability(
            "substrate.atspi_click",
            "kria-gui-runtime",
            CapabilityRiskLevel::High,
            &["wrong_ui_target", "focus_race"],
        ),
        capability(
            "verifier.artifact_content",
            "kria-verifier",
            CapabilityRiskLevel::High,
            &["false_success", "artifact_content_mismatch"],
        ),
        capability(
            "verifier.false_success_guard",
            "kria-verifier",
            CapabilityRiskLevel::Critical,
            &["hallucinated_completion", "unsupported_success_claim"],
        ),
        capability(
            "safety.retrieval_isolation",
            "kria-safety",
            CapabilityRiskLevel::High,
            &["retrieval_leakage", "cloud_llm_leakage"],
        ),
        capability(
            "safety.security_invariant",
            "kria-safety",
            CapabilityRiskLevel::Critical,
            &["policy_bypass", "unsafe_side_effect"],
        ),
        capability(
            "safety.destructive_vm_isolation",
            "kria-safety",
            CapabilityRiskLevel::Critical,
            &[
                "host_destructive_execution",
                "missing_vm_snapshot",
                "destructive_policy_bypass",
            ],
        ),
        capability(
            "environment.display_compat",
            "kria-gui-runtime",
            CapabilityRiskLevel::High,
            &["wayland_incompatibility", "x11_assumption"],
        ),
        capability(
            "recovery.resilience",
            "kria-workflow-runtime",
            CapabilityRiskLevel::High,
            &["retry_loop", "degraded_runtime_failure"],
        ),
        capability(
            "hitl.timeline",
            "kria-workflow-runtime",
            CapabilityRiskLevel::Critical,
            &["stale_decision_execution", "unsafe_approval"],
        ),
        capability(
            "llm.cognition_advisory",
            "kria-eval",
            CapabilityRiskLevel::Medium,
            &[
                "model_variance",
                "prompt_memorization",
                "provider_specific_behavior",
            ],
        ),
        capability(
            "eval.general_gui_regression",
            "kria-eval",
            CapabilityRiskLevel::Low,
            &["generic_gui_regression"],
        ),
    ]
}

pub fn derive_governance_metadata(
    id: &str,
    description: &str,
    prompt: &str,
    behavior: &ExpectedBehavior,
    display_server: DisplayServerRequirement,
    requires_desktop: bool,
    tags: &[String],
) -> EvalGovernanceMetadata {
    let haystack = normalized_haystack(id, description, prompt, tags);
    let required_tools = lower_set(&behavior.required_tools);
    let forbidden_tools = lower_set(&behavior.forbidden_tools);
    let substrate = behavior
        .substrate
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut capabilities = Vec::new();
    let mut failure_modes = Vec::new();

    if required_tools.contains("write_file") || substrate.contains("filewritethenopen") {
        push_unique(&mut capabilities, "substrate.file_write_open");
        push_unique(&mut failure_modes, "artifact_missing");
    }

    if !behavior.expected_artifacts.is_empty() {
        push_unique(&mut capabilities, "verifier.artifact_content");
        push_unique(&mut failure_modes, "artifact_content_mismatch");
    }

    if required_tools.contains("execute_bash")
        || substrate.contains("terminal")
        || haystack.contains("terminal-execution")
        || haystack.contains("run-intent")
    {
        push_unique(&mut capabilities, "substrate.terminal_execution");
        push_unique(&mut failure_modes, "command_not_executed");
    }

    if required_tools.contains("browser_search")
        || required_tools.contains("managed_browser_navigate")
        || substrate.contains("browser")
        || haystack.contains("browser")
    {
        push_unique(&mut capabilities, "substrate.browser_cdp");
        push_unique(&mut failure_modes, "wrong_browser_state");
    }

    if required_tools.contains("click_ui_element")
        || haystack.contains("atspi")
        || haystack.contains("interaction-heavy")
    {
        push_unique(&mut capabilities, "substrate.atspi_click");
        push_unique(&mut failure_modes, "wrong_ui_target");
    }

    if required_tools.contains("open_application")
        || required_tools.contains("open_application_with_file")
        || substrate.contains("appopen")
        || haystack.contains("app-open")
    {
        push_unique(&mut capabilities, "substrate.app_lifecycle");
        push_unique(&mut failure_modes, "app_not_opened");
    }

    if forbidden_tools.iter().any(|tool| {
        matches!(
            tool.as_str(),
            "web_search" | "search_news" | "searxng_search" | "retrieve_memory"
        )
    }) || haystack.contains("retrieval-isolation")
    {
        push_unique(&mut capabilities, "safety.retrieval_isolation");
        push_unique(&mut failure_modes, "retrieval_leakage");
    }

    if !behavior.forbidden_response_patterns.is_empty()
        || haystack.contains("false-success")
        || haystack.contains("completion-truth")
    {
        push_unique(&mut capabilities, "verifier.false_success_guard");
        push_unique(&mut failure_modes, "hallucinated_completion");
    }

    if haystack.contains("language-detection")
        || haystack.contains("python")
        || haystack.contains("javascript")
        || haystack.contains("rust")
        || haystack.contains("go")
    {
        push_unique(&mut capabilities, "intent.language_detection");
        push_unique(&mut failure_modes, "wrong_artifact_language");
    }

    if haystack.contains("app-resolution")
        || haystack.contains("conjunction")
        || haystack.contains("alias")
    {
        push_unique(&mut capabilities, "intent.app_alias");
        push_unique(&mut failure_modes, "wrong_app_target");
    }

    if haystack.contains("multi")
        || haystack.contains("workflow")
        || haystack.contains("semantic")
        || haystack.contains("cognition")
    {
        push_unique(&mut capabilities, "intent.multi_step_gui");
        push_unique(&mut failure_modes, "partial_completion");
    }

    if haystack.contains("wayland")
        || haystack.contains("x11")
        || matches!(
            display_server,
            DisplayServerRequirement::X11Only
                | DisplayServerRequirement::WaylandOnly
                | DisplayServerRequirement::X11OrWayland
        )
    {
        push_unique(&mut capabilities, "environment.display_compat");
        push_unique(&mut failure_modes, "display_incompatibility");
    }

    if haystack.contains("chaos")
        || haystack.contains("recovery")
        || haystack.contains("graceful")
        || haystack.contains("degraded")
    {
        push_unique(&mut capabilities, "recovery.resilience");
        push_unique(&mut failure_modes, "degraded_runtime_failure");
    }

    if haystack.contains("hitl")
        || haystack.contains("approval")
        || haystack.contains("stale")
        || haystack.contains("resume")
    {
        push_unique(&mut capabilities, "hitl.timeline");
        push_unique(&mut failure_modes, "stale_decision_execution");
    }

    if haystack.contains("security")
        || haystack.contains("destructive")
        || haystack.contains("dangerous")
        || haystack.contains("host-mutating")
        || haystack.contains("policy")
    {
        push_unique(&mut capabilities, "safety.security_invariant");
        push_unique(&mut failure_modes, "unsafe_side_effect");
    }

    if capabilities.is_empty() {
        push_unique(&mut capabilities, "eval.general_gui_regression");
        push_unique(&mut failure_modes, "generic_gui_regression");
    }

    let priority = derive_priority(&capabilities, &failure_modes, &haystack);
    let cost_class = derive_cost_class(&haystack, requires_desktop, &priority);
    let environment_profile =
        derive_environment_profile(&haystack, display_server, requires_desktop);
    let oracle_type = derive_oracle_type(&capabilities, &required_tools, behavior, &haystack);
    let owner = derive_owner(&capabilities);
    let lifecycle = match priority {
        EvalPriority::P0SafetyCritical | EvalPriority::P1ReleaseCriticalRuntime => {
            EvalLifecycleState::ReleaseCritical
        }
        _ => EvalLifecycleState::Regression,
    };
    let cleanup_contract = derive_cleanup_contract(&haystack, &environment_profile);
    let max_runtime_ms = max_runtime_for(&cost_class);
    let dedup_key = dedup_key_for(
        &capabilities,
        &failure_modes,
        &oracle_type,
        &environment_profile,
    );

    EvalGovernanceMetadata {
        capability_ids: capabilities,
        failure_mode_ids: failure_modes,
        priority: Some(priority),
        cost_class: Some(cost_class),
        environment_profile: Some(environment_profile),
        oracle_type: Some(oracle_type),
        owner: Some(owner),
        lifecycle: Some(lifecycle),
        cleanup_contract: Some(cleanup_contract),
        max_runtime_ms: Some(max_runtime_ms),
        dedup_key: Some(dedup_key),
    }
}

pub fn build_governance_report(cases: &[GuiEvalCase]) -> GovernanceReport {
    let mut capability_cases: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut by_cost: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut by_priority: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut by_dedup: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut missing_metadata_cases = Vec::new();
    let mut ownerless_count = 0;
    let mut low_oracle_strength_count = 0;

    for case in cases {
        let meta = &case.governance;
        if meta.capability_ids.is_empty()
            || meta.failure_mode_ids.is_empty()
            || meta.priority.is_none()
            || meta.cost_class.is_none()
            || meta.environment_profile.is_none()
            || meta.oracle_type.is_none()
            || meta.owner.as_deref().unwrap_or_default().is_empty()
            || meta.dedup_key.as_deref().unwrap_or_default().is_empty()
        {
            missing_metadata_cases.push(case.id.clone());
        }

        if meta.owner.as_deref().unwrap_or_default().is_empty() {
            ownerless_count += 1;
        }

        for capability in &meta.capability_ids {
            capability_cases
                .entry(capability.clone())
                .or_default()
                .push(case.id.clone());
        }

        if let Some(cost) = &meta.cost_class {
            by_cost
                .entry(cost.as_str().to_string())
                .or_default()
                .push(case.id.clone());
        }

        if let Some(priority) = &meta.priority {
            by_priority
                .entry(priority.as_str().to_string())
                .or_default()
                .push(case.id.clone());
        }

        if let Some(dedup_key) = &meta.dedup_key {
            by_dedup
                .entry(dedup_key.clone())
                .or_default()
                .push(case.id.clone());
        }

        if matches!(meta.oracle_type, Some(EvalOracleType::ToolTrace))
            && matches!(
                meta.priority,
                Some(EvalPriority::P0SafetyCritical | EvalPriority::P1ReleaseCriticalRuntime)
            )
        {
            low_oracle_strength_count += 1;
        }
    }

    let registry = capability_registry();
    let mut definitions: BTreeMap<String, CapabilityDefinition> = registry
        .into_iter()
        .map(|definition| (definition.id.clone(), definition))
        .collect();
    for capability_id in capability_cases.keys() {
        definitions.entry(capability_id.clone()).or_insert_with(|| {
            capability(capability_id, "kria-eval", CapabilityRiskLevel::Medium, &[])
        });
    }

    let capabilities = definitions
        .into_values()
        .map(|definition| {
            let mut linked_eval_ids = capability_cases
                .get(&definition.id)
                .cloned()
                .unwrap_or_default();
            linked_eval_ids.sort();
            let linked_eval_count = linked_eval_ids.len();
            linked_eval_ids.truncate(COVERAGE_CASE_LIMIT);
            CapabilityCoverageEntry {
                capability_id: definition.id,
                owner: definition.owner,
                risk_level: definition.risk_level,
                protected_failure_modes: definition.protected_failure_modes,
                linked_eval_count,
                linked_eval_ids,
            }
        })
        .collect();

    let cost_breakdown = breakdown(by_cost);
    let priority_breakdown = breakdown(by_priority);

    let mut duplicate_groups: Vec<(String, Vec<String>)> = by_dedup
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .collect();
    duplicate_groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    let duplicate_group_count = duplicate_groups.len();
    let duplicate_case_count: usize = duplicate_groups
        .iter()
        .map(|(_, ids)| ids.len().saturating_sub(1))
        .sum();
    let duplicate_dedup_keys = duplicate_groups
        .into_iter()
        .take(DEDUP_GROUP_LIMIT)
        .map(|(dedup_key, mut case_ids)| {
            let duplicate_case_count = case_ids.len().saturating_sub(1);
            case_ids.sort();
            case_ids.truncate(DEDUP_CASE_LIMIT);
            DedupFinding {
                dedup_key,
                duplicate_case_count,
                case_ids,
            }
        })
        .collect();

    let total_cases = cases.len();
    let duplicate_ratio = ratio(duplicate_case_count, total_cases);
    let missing_ratio = ratio(missing_metadata_cases.len(), total_cases);
    let ownerless_ratio = ratio(ownerless_count, total_cases);
    let low_oracle_ratio = ratio(low_oracle_strength_count, total_cases);
    let entropy_score = duplicate_ratio + missing_ratio + ownerless_ratio + low_oracle_ratio;

    GovernanceReport {
        capabilities,
        cost_breakdown,
        priority_breakdown,
        duplicate_dedup_keys,
        entropy: GovernanceEntropy {
            total_cases,
            duplicate_group_count,
            duplicate_case_count,
            duplicate_ratio,
            missing_metadata_count: missing_metadata_cases.len(),
            ownerless_count,
            low_oracle_strength_count,
            entropy_score,
        },
        missing_metadata_cases,
    }
}

fn capability(
    id: &str,
    owner: &str,
    risk_level: CapabilityRiskLevel,
    protected_failure_modes: &[&str],
) -> CapabilityDefinition {
    CapabilityDefinition {
        id: id.to_string(),
        owner: owner.to_string(),
        risk_level,
        protected_failure_modes: protected_failure_modes
            .iter()
            .map(|mode| mode.to_string())
            .collect(),
    }
}

fn derive_priority(
    capabilities: &[String],
    failure_modes: &[String],
    haystack: &str,
) -> EvalPriority {
    if capabilities.iter().any(|capability| {
        matches!(
            capability.as_str(),
            "safety.security_invariant" | "hitl.timeline" | "verifier.false_success_guard"
        )
    }) || failure_modes.iter().any(|mode| {
        matches!(
            mode.as_str(),
            "unsafe_side_effect" | "stale_decision_execution"
        )
    }) {
        return EvalPriority::P0SafetyCritical;
    }

    if capabilities.iter().any(|capability| {
        matches!(
            capability.as_str(),
            "safety.retrieval_isolation"
                | "verifier.artifact_content"
                | "environment.display_compat"
        )
    }) || haystack.contains("hardening")
        || haystack.contains("production")
    {
        return EvalPriority::P1ReleaseCriticalRuntime;
    }

    if haystack.contains("llm") || haystack.contains("provider") || haystack.contains("model") {
        return EvalPriority::P4ProviderModelAdvisory;
    }

    if capabilities
        .iter()
        .any(|capability| capability == "intent.multi_step_gui")
        || haystack.contains("cognition")
    {
        return EvalPriority::P3WorkflowCognition;
    }

    EvalPriority::P2CoreGuiRegression
}

fn derive_cost_class(
    haystack: &str,
    requires_desktop: bool,
    priority: &EvalPriority,
) -> EvalCostClass {
    if haystack.contains("destructive")
        || haystack.contains("vm-only")
        || haystack.contains("dangerous")
    {
        return EvalCostClass::C6DestructiveVm;
    }
    if haystack.contains("hitl")
        || haystack.contains("resume")
        || haystack.contains("long-horizon")
        || haystack.contains("stale")
    {
        return EvalCostClass::C5LongHorizonHitl;
    }
    if haystack.contains("llm") || haystack.contains("provider") || haystack.contains("model") {
        return EvalCostClass::C4WithLlmCognition;
    }
    if matches!(
        priority,
        EvalPriority::P0SafetyCritical | EvalPriority::P1ReleaseCriticalRuntime
    ) || haystack.contains("chaos")
        || haystack.contains("atspi")
        || haystack.contains("interaction-heavy")
        || haystack.contains("production")
        || haystack.contains("hardening")
    {
        return EvalCostClass::C3FullGuiRegression;
    }
    if requires_desktop {
        return EvalCostClass::C2LocalGuiSmoke;
    }

    EvalCostClass::C1FastNoDisplay
}

fn derive_environment_profile(
    haystack: &str,
    display_server: DisplayServerRequirement,
    requires_desktop: bool,
) -> EvalEnvironmentProfile {
    if haystack.contains("destructive")
        || haystack.contains("vm-only")
        || haystack.contains("dangerous")
    {
        return EvalEnvironmentProfile::VmSnapshot;
    }

    if !requires_desktop {
        return EvalEnvironmentProfile::HostNoDisplay;
    }

    match display_server {
        DisplayServerRequirement::X11Only => EvalEnvironmentProfile::HostGuiX11,
        DisplayServerRequirement::WaylandOnly => EvalEnvironmentProfile::HostGuiWayland,
        DisplayServerRequirement::Any | DisplayServerRequirement::X11OrWayland => {
            EvalEnvironmentProfile::HostGuiAny
        }
    }
}

fn derive_oracle_type(
    capabilities: &[String],
    required_tools: &BTreeSet<String>,
    behavior: &ExpectedBehavior,
    haystack: &str,
) -> EvalOracleType {
    if capabilities.iter().any(|capability| {
        matches!(
            capability.as_str(),
            "safety.security_invariant"
                | "verifier.false_success_guard"
                | "safety.retrieval_isolation"
                | "hitl.timeline"
        )
    }) {
        return EvalOracleType::CompositeInvariant;
    }

    if !behavior.expected_artifacts.is_empty() {
        return EvalOracleType::ArtifactContent;
    }

    if capabilities
        .iter()
        .any(|capability| capability == "substrate.browser_cdp")
    {
        return EvalOracleType::BrowserState;
    }

    if capabilities
        .iter()
        .any(|capability| capability == "substrate.atspi_click")
        || haystack.contains("semantic")
        || haystack.contains("interaction")
    {
        return EvalOracleType::GuiSemanticState;
    }

    if required_tools.contains("open_application")
        || required_tools.contains("open_application_with_file")
    {
        return EvalOracleType::ProcessState;
    }

    EvalOracleType::ToolTrace
}

fn derive_owner(capabilities: &[String]) -> String {
    for capability in capabilities {
        if capability.starts_with("safety.") {
            return "kria-safety".to_string();
        }
        if capability.starts_with("verifier.") {
            return "kria-verifier".to_string();
        }
        if capability.starts_with("hitl.") || capability.starts_with("recovery.") {
            return "kria-workflow-runtime".to_string();
        }
    }
    for capability in capabilities {
        if capability.starts_with("substrate.")
            || capability.starts_with("intent.")
            || capability.starts_with("environment.")
        {
            return "kria-gui-runtime".to_string();
        }
    }
    "kria-eval".to_string()
}

fn derive_cleanup_contract(haystack: &str, environment_profile: &EvalEnvironmentProfile) -> String {
    if matches!(environment_profile, EvalEnvironmentProfile::VmSnapshot) {
        return "vm_snapshot_restore_required".to_string();
    }
    if haystack.contains("browser") {
        return "browser_profile_scoped".to_string();
    }
    "generated_artifacts_only".to_string()
}

fn max_runtime_for(cost_class: &EvalCostClass) -> u64 {
    match cost_class {
        EvalCostClass::C0FastDeterministic => 10_000,
        EvalCostClass::C1FastNoDisplay => 30_000,
        EvalCostClass::C2LocalGuiSmoke => 45_000,
        EvalCostClass::C3FullGuiRegression => 90_000,
        EvalCostClass::C4WithLlmCognition => 120_000,
        EvalCostClass::C5LongHorizonHitl => 300_000,
        EvalCostClass::C6DestructiveVm => 600_000,
    }
}

fn normalized_haystack(id: &str, description: &str, prompt: &str, tags: &[String]) -> String {
    let mut parts = vec![
        id.to_ascii_lowercase(),
        description.to_ascii_lowercase(),
        prompt.to_ascii_lowercase(),
    ];
    parts.extend(tags.iter().map(|tag| tag.to_ascii_lowercase()));
    parts.join(" ")
}

fn lower_set(values: &[String]) -> BTreeSet<String> {
    values
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect()
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn dedup_key_for(
    capabilities: &[String],
    failure_modes: &[String],
    oracle_type: &EvalOracleType,
    environment_profile: &EvalEnvironmentProfile,
) -> String {
    format!(
        "{}|{}|{}|{}",
        sorted_join(capabilities),
        sorted_join(failure_modes),
        oracle_type.as_str(),
        environment_profile.as_str()
    )
}

fn sorted_join(values: &[String]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort();
    sorted.dedup();
    sorted.join("+")
}

fn breakdown(groups: BTreeMap<String, Vec<String>>) -> Vec<GovernanceBreakdown> {
    groups
        .into_iter()
        .map(|(key, mut case_ids)| {
            case_ids.sort();
            GovernanceBreakdown {
                key,
                count: case_ids.len(),
                case_ids,
            }
        })
        .collect()
}

fn ratio(count: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        count as f32 / total as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui_eval::types::{ExpectedArtifact, ExpectedBehavior};

    fn behavior() -> ExpectedBehavior {
        ExpectedBehavior {
            substrate: Some("FileWriteThenOpen".to_string()),
            expected_artifacts: vec![ExpectedArtifact {
                path_pattern: "hello_*.py".to_string(),
                content_contains: Some("print".to_string()),
                min_size_bytes: Some(1),
            }],
            required_tools: vec![
                "write_file".to_string(),
                "open_application_with_file".to_string(),
            ],
            forbidden_tools: vec!["web_search".to_string()],
            forbidden_response_patterns: Vec::new(),
            required_response_patterns: Vec::new(),
            expect_success: true,
            app_already_running: false,
        }
    }

    #[test]
    fn derives_artifact_and_retrieval_metadata() {
        let tags = vec![
            "file-substrate".to_string(),
            "retrieval-isolation".to_string(),
        ];
        let metadata = derive_governance_metadata(
            "unit-001",
            "write file without retrieval",
            "open code and write python",
            &behavior(),
            DisplayServerRequirement::Any,
            true,
            &tags,
        );

        assert!(metadata
            .capability_ids
            .contains(&"substrate.file_write_open".to_string()));
        assert!(metadata
            .capability_ids
            .contains(&"verifier.artifact_content".to_string()));
        assert!(metadata
            .capability_ids
            .contains(&"safety.retrieval_isolation".to_string()));
        assert!(metadata
            .dedup_key
            .as_deref()
            .unwrap_or_default()
            .contains('|'));
    }

    #[test]
    fn governance_report_marks_duplicate_dedup_keys() {
        let tags = vec!["file-substrate".to_string()];
        let metadata = derive_governance_metadata(
            "unit-001",
            "write file",
            "write python",
            &behavior(),
            DisplayServerRequirement::Any,
            true,
            &tags,
        );
        let case_a = GuiEvalCase {
            id: "unit-a".to_string(),
            description: "a".to_string(),
            prompt: "write python".to_string(),
            expected_behavior: behavior(),
            display_server: DisplayServerRequirement::Any,
            tags: tags.clone(),
            requires_desktop: true,
            timeout: std::time::Duration::from_secs(1),
            governance: metadata.clone(),
        };
        let case_b = GuiEvalCase {
            id: "unit-b".to_string(),
            description: "b".to_string(),
            prompt: "write python again".to_string(),
            expected_behavior: behavior(),
            display_server: DisplayServerRequirement::Any,
            tags,
            requires_desktop: true,
            timeout: std::time::Duration::from_secs(1),
            governance: metadata,
        };

        let report = build_governance_report(&[case_a, case_b]);
        assert_eq!(report.entropy.duplicate_group_count, 1);
        assert_eq!(report.entropy.duplicate_case_count, 1);
        assert!(report.missing_metadata_cases.is_empty());
    }
}
