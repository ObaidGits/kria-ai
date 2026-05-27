//! Sampled with-LLM GUI cognition matrix.
//!
//! Phase 7 is intentionally advisory-first. This module does not create a model
//! leaderboard and does not use LLM output as a correctness oracle. It selects a
//! tiny set of existing GUI cognition cases, applies provider/model budget
//! limits, and reports which cells are eligible for future live cognition runs.
//! Structural policy, verifier, and GUI oracles remain authoritative.

use serde::{Deserialize, Serialize};

use super::governance::{
    derive_governance_metadata, EvalCostClass, EvalLifecycleState, EvalPriority,
};
use super::gui_cognition_suite::all_gui_cognition_cases;
use super::types::GuiEvalCase;

const DEFAULT_MAX_CASES: usize = 4;
const HARD_MAX_CASES: usize = 6;
const DEFAULT_MAX_PROFILES: usize = 1;
const HARD_MAX_PROFILES: usize = 3;
const DEFAULT_MAX_REQUESTS: usize = 4;
const HARD_MAX_REQUESTS: usize = 12;
const DEFAULT_MAX_PROMPT_TOKENS: usize = 12_000;
const DEFAULT_MAX_COMPLETION_TOKENS: usize = 2_048;
const DEFAULT_MAX_RUNTIME_MS: u64 = 120_000;

const LLM_COGNITION_CASE_IDS: &[&str] = &[
    "cog-code-001-number-table",
    "cog-run-001-number-table-output",
    "cog-browser-001-youtube-playlist",
    "cog-recovery-001-app-not-installed",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderClass {
    WeakLocal,
    MediumLocal,
    StrongCloud,
}

impl LlmProviderClass {
    pub fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "weak-local" | "weak_local" | "local-weak" | "local_weak" => Some(Self::WeakLocal),
            "medium-local" | "medium_local" | "local-medium" | "local_medium" => {
                Some(Self::MediumLocal)
            }
            "strong-cloud" | "strong_cloud" | "cloud-strong" | "cloud_strong" => {
                Some(Self::StrongCloud)
            }
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::WeakLocal => "weak_local",
            Self::MediumLocal => "medium_local",
            Self::StrongCloud => "strong_cloud",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCognitionBudget {
    pub max_cases: usize,
    pub max_profiles: usize,
    pub max_requests: usize,
    pub max_prompt_tokens: usize,
    pub max_completion_tokens: usize,
    pub max_runtime_ms: u64,
}

impl LlmCognitionBudget {
    pub fn from_env() -> Self {
        let max_cases =
            env_usize("KRIA_EVAL_LLM_MAX_CASES", DEFAULT_MAX_CASES).clamp(1, HARD_MAX_CASES);
        let max_profiles = env_usize("KRIA_EVAL_LLM_MAX_PROFILES", DEFAULT_MAX_PROFILES)
            .clamp(1, HARD_MAX_PROFILES);
        let default_requests = DEFAULT_MAX_REQUESTS.min(max_cases.saturating_mul(max_profiles));
        let max_requests =
            env_usize("KRIA_EVAL_LLM_MAX_REQUESTS", default_requests).clamp(1, HARD_MAX_REQUESTS);
        let max_prompt_tokens =
            env_usize("KRIA_EVAL_LLM_MAX_PROMPT_TOKENS", DEFAULT_MAX_PROMPT_TOKENS);
        let max_completion_tokens = env_usize(
            "KRIA_EVAL_LLM_MAX_COMPLETION_TOKENS",
            DEFAULT_MAX_COMPLETION_TOKENS,
        );
        let max_runtime_ms = env_u64("KRIA_EVAL_LLM_MAX_RUNTIME_MS", DEFAULT_MAX_RUNTIME_MS);

        Self {
            max_cases,
            max_profiles,
            max_requests,
            max_prompt_tokens,
            max_completion_tokens,
            max_runtime_ms,
        }
    }
}

impl Default for LlmCognitionBudget {
    fn default() -> Self {
        Self {
            max_cases: DEFAULT_MAX_CASES,
            max_profiles: DEFAULT_MAX_PROFILES,
            max_requests: DEFAULT_MAX_REQUESTS,
            max_prompt_tokens: DEFAULT_MAX_PROMPT_TOKENS,
            max_completion_tokens: DEFAULT_MAX_COMPLETION_TOKENS,
            max_runtime_ms: DEFAULT_MAX_RUNTIME_MS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProviderProfile {
    pub id: String,
    pub provider_class: LlmProviderClass,
    pub model: String,
    pub endpoint_configured: bool,
    pub opt_in_enabled: bool,
    pub advisory_only: bool,
}

impl LlmProviderProfile {
    pub fn from_env_profiles(budget: &LlmCognitionBudget) -> Vec<Self> {
        let opt_in_enabled = std::env::var("KRIA_EVAL_LLM_COGNITION").as_deref() == Ok("1");
        let endpoint_configured = std::env::var("KRIA_EVAL_LLM_BASE_URL")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
            || std::env::var("KRIA_EVAL_LLM_CMD")
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
            || std::env::var("KRIA_EVAL_LLM_PROFILE_READY").as_deref() == Ok("1");

        if let Ok(spec) = std::env::var("KRIA_EVAL_LLM_PROFILES") {
            let profiles = spec
                .split(';')
                .filter_map(|entry| parse_profile_spec(entry, opt_in_enabled, endpoint_configured))
                .take(budget.max_profiles)
                .collect::<Vec<_>>();
            if !profiles.is_empty() {
                return profiles;
            }
        }

        let id = std::env::var("KRIA_EVAL_LLM_PROFILE_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "local-weak-default".to_string());
        let provider_class = std::env::var("KRIA_EVAL_LLM_PROVIDER_CLASS")
            .ok()
            .and_then(|value| LlmProviderClass::from_str(&value))
            .unwrap_or(LlmProviderClass::WeakLocal);
        let model = std::env::var("KRIA_EVAL_LLM_MODEL")
            .ok()
            .or_else(|| std::env::var("KRIA_EVAL_ACTIVE_MODEL").ok())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "unset".to_string());

        vec![Self {
            id,
            provider_class,
            model,
            endpoint_configured,
            opt_in_enabled,
            advisory_only: true,
        }]
    }

    fn ready(&self) -> bool {
        self.opt_in_enabled && self.endpoint_configured && !self.model.trim().is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmCognitionCellStatus {
    AdvisoryReady,
    BlockedByProvider,
    BlockedByBudget,
    StructuralBlocked,
}

impl LlmCognitionCellStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AdvisoryReady => "advisory_ready",
            Self::BlockedByProvider => "blocked_by_provider",
            Self::BlockedByBudget => "blocked_by_budget",
            Self::StructuralBlocked => "structural_blocked",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCognitionMatrixCell {
    pub case_id: String,
    pub profile_id: String,
    pub provider_class: LlmProviderClass,
    pub model: String,
    pub status: LlmCognitionCellStatus,
    pub advisory_only: bool,
    pub estimated_prompt_tokens: usize,
    pub max_completion_tokens: usize,
    pub structural_authority: String,
    pub reason: String,
    pub capability_ids: Vec<String>,
    pub failure_mode_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCognitionMatrixSummary {
    pub sampled_cases: usize,
    pub provider_profiles: usize,
    pub total_cells: usize,
    pub advisory_ready: usize,
    pub blocked_by_provider: usize,
    pub blocked_by_budget: usize,
    pub structural_blocked: usize,
    pub estimated_request_count: usize,
    pub estimated_prompt_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCognitionMatrixReport {
    pub run_id: String,
    pub generated_at: String,
    pub budget: LlmCognitionBudget,
    pub summary: LlmCognitionMatrixSummary,
    pub profiles: Vec<LlmProviderProfile>,
    pub sampled_cases: Vec<String>,
    pub cells: Vec<LlmCognitionMatrixCell>,
}

pub fn run_llm_cognition_matrix(run_id: impl Into<String>) -> LlmCognitionMatrixReport {
    let budget = LlmCognitionBudget::from_env();
    let profiles = LlmProviderProfile::from_env_profiles(&budget);
    run_llm_cognition_matrix_with(run_id, budget, profiles)
}

pub fn run_llm_cognition_matrix_with(
    run_id: impl Into<String>,
    budget: LlmCognitionBudget,
    profiles: Vec<LlmProviderProfile>,
) -> LlmCognitionMatrixReport {
    let profiles = profiles
        .into_iter()
        .take(budget.max_profiles)
        .collect::<Vec<_>>();
    let cases = sampled_llm_cognition_cases(budget.max_cases);
    let mut cells = Vec::new();
    let mut estimated_request_count = 0usize;

    for case in &cases {
        for profile in &profiles {
            estimated_request_count = estimated_request_count.saturating_add(1);
            cells.push(evaluate_cell(
                case,
                profile,
                &budget,
                estimated_request_count,
            ));
        }
    }

    let summary = summarize_cells(&cases, &profiles, &cells);
    LlmCognitionMatrixReport {
        run_id: run_id.into(),
        generated_at: unix_now(),
        budget,
        summary,
        profiles,
        sampled_cases: cases.iter().map(|case| case.id.clone()).collect(),
        cells,
    }
}

pub fn sampled_llm_cognition_cases(max_cases: usize) -> Vec<GuiEvalCase> {
    let source = all_gui_cognition_cases();
    LLM_COGNITION_CASE_IDS
        .iter()
        .filter_map(|id| source.iter().find(|case| case.id == *id).cloned())
        .take(max_cases.min(HARD_MAX_CASES))
        .map(mark_case_as_llm_advisory)
        .collect()
}

pub fn print_llm_cognition_matrix_report(report: &LlmCognitionMatrixReport) {
    println!("── With-LLM Cognition Matrix ──────────────────────────────────");
    println!("  Run ID:              {}", report.run_id);
    println!("  Sampled Cases:       {}", report.summary.sampled_cases);
    println!(
        "  Provider Profiles:   {}",
        report.summary.provider_profiles
    );
    println!("  Total Cells:         {}", report.summary.total_cells);
    println!("  Advisory Ready:      {}", report.summary.advisory_ready);
    println!(
        "  Blocked Provider:    {}",
        report.summary.blocked_by_provider
    );
    println!(
        "  Blocked Budget:      {}",
        report.summary.blocked_by_budget
    );
    println!(
        "  Structural Blocked:  {}",
        report.summary.structural_blocked
    );
    println!(
        "  Estimated Requests:  {}",
        report.summary.estimated_request_count
    );
    println!(
        "  Estimated Tokens:    {}",
        report.summary.estimated_prompt_tokens
    );
    for cell in &report.cells {
        println!(
            "  {} [{}/{}] {}",
            status_icon(cell.status),
            cell.case_id,
            cell.profile_id,
            cell.status.as_str()
        );
        if cell.status != LlmCognitionCellStatus::AdvisoryReady {
            println!("     {}", cell.reason);
        }
    }
    println!();
}

fn mark_case_as_llm_advisory(mut case: GuiEvalCase) -> GuiEvalCase {
    push_tag(&mut case, "llm-cognition-matrix");
    push_tag(&mut case, "provider-model-advisory");
    case.governance = derive_governance_metadata(
        &case.id,
        &case.description,
        &case.prompt,
        &case.expected_behavior,
        case.display_server,
        case.requires_desktop,
        &case.tags,
    );
    push_unique(
        &mut case.governance.capability_ids,
        "llm.cognition_advisory",
    );
    push_unique(&mut case.governance.failure_mode_ids, "model_variance");
    case.governance.priority = Some(EvalPriority::P4ProviderModelAdvisory);
    case.governance.cost_class = Some(EvalCostClass::C4WithLlmCognition);
    case.governance.lifecycle = Some(EvalLifecycleState::Experimental);
    case.governance.owner = Some("kria-eval".to_string());
    case.governance.dedup_key = Some(format!(
        "llm.cognition_advisory|{}|{}",
        sorted_join(&case.governance.capability_ids),
        case.governance
            .oracle_type
            .as_ref()
            .map(|oracle| oracle.as_str())
            .unwrap_or("unknown")
    ));
    case
}

fn evaluate_cell(
    case: &GuiEvalCase,
    profile: &LlmProviderProfile,
    budget: &LlmCognitionBudget,
    request_index: usize,
) -> LlmCognitionMatrixCell {
    let estimated_prompt_tokens = estimate_prompt_tokens(case);
    let (status, reason) = if !has_structural_oracle(case) {
        (
            LlmCognitionCellStatus::StructuralBlocked,
            "case lacks structural governance/oracle metadata; LLM output cannot be evaluated safely"
                .to_string(),
        )
    } else if request_index > budget.max_requests
        || estimated_prompt_tokens > budget.max_prompt_tokens
    {
        (
            LlmCognitionCellStatus::BlockedByBudget,
            "cell exceeds configured request or prompt-token budget".to_string(),
        )
    } else if !profile.ready() {
        (
            LlmCognitionCellStatus::BlockedByProvider,
            "provider not opted in or no endpoint/fixture configured; advisory cell not executed"
                .to_string(),
        )
    } else {
        (
            LlmCognitionCellStatus::AdvisoryReady,
            "eligible for advisory with-LLM cognition run; deterministic oracles remain authoritative"
                .to_string(),
        )
    };

    LlmCognitionMatrixCell {
        case_id: case.id.clone(),
        profile_id: profile.id.clone(),
        provider_class: profile.provider_class,
        model: profile.model.clone(),
        status,
        advisory_only: true,
        estimated_prompt_tokens,
        max_completion_tokens: budget.max_completion_tokens,
        structural_authority: "policy_verifier_gui_oracle_wins".to_string(),
        reason,
        capability_ids: case.governance.capability_ids.clone(),
        failure_mode_ids: case.governance.failure_mode_ids.clone(),
    }
}

fn summarize_cells(
    cases: &[GuiEvalCase],
    profiles: &[LlmProviderProfile],
    cells: &[LlmCognitionMatrixCell],
) -> LlmCognitionMatrixSummary {
    let advisory_ready = cells
        .iter()
        .filter(|cell| cell.status == LlmCognitionCellStatus::AdvisoryReady)
        .count();
    let blocked_by_provider = cells
        .iter()
        .filter(|cell| cell.status == LlmCognitionCellStatus::BlockedByProvider)
        .count();
    let blocked_by_budget = cells
        .iter()
        .filter(|cell| cell.status == LlmCognitionCellStatus::BlockedByBudget)
        .count();
    let structural_blocked = cells
        .iter()
        .filter(|cell| cell.status == LlmCognitionCellStatus::StructuralBlocked)
        .count();
    let estimated_prompt_tokens = cells
        .iter()
        .map(|cell| cell.estimated_prompt_tokens)
        .sum::<usize>();

    LlmCognitionMatrixSummary {
        sampled_cases: cases.len(),
        provider_profiles: profiles.len(),
        total_cells: cells.len(),
        advisory_ready,
        blocked_by_provider,
        blocked_by_budget,
        structural_blocked,
        estimated_request_count: cells.len(),
        estimated_prompt_tokens,
    }
}

fn has_structural_oracle(case: &GuiEvalCase) -> bool {
    !case.governance.capability_ids.is_empty()
        && !case.governance.failure_mode_ids.is_empty()
        && case.governance.oracle_type.is_some()
        && case.governance.cost_class == Some(EvalCostClass::C4WithLlmCognition)
        && case.governance.priority == Some(EvalPriority::P4ProviderModelAdvisory)
}

fn parse_profile_spec(
    spec: &str,
    opt_in_enabled: bool,
    endpoint_configured: bool,
) -> Option<LlmProviderProfile> {
    let parts = spec.split(':').map(str::trim).collect::<Vec<_>>();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let provider_class = LlmProviderClass::from_str(parts[1])?;
    Some(LlmProviderProfile {
        id: parts[0].to_string(),
        provider_class,
        model: parts[2].to_string(),
        endpoint_configured,
        opt_in_enabled,
        advisory_only: true,
    })
}

fn estimate_prompt_tokens(case: &GuiEvalCase) -> usize {
    let text = format!(
        "{}\n{}\n{}",
        case.description,
        case.prompt,
        case.expected_behavior.required_response_patterns.join("\n")
    );
    text.len().div_ceil(4).max(1)
}

fn push_tag(case: &mut GuiEvalCase, tag: &str) {
    if !case.tags.iter().any(|existing| existing == tag) {
        case.tags.push(tag.to_string());
    }
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn sorted_join(values: &[String]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort();
    sorted.dedup();
    sorted.join("+")
}

fn status_icon(status: LlmCognitionCellStatus) -> &'static str {
    match status {
        LlmCognitionCellStatus::AdvisoryReady => "READY",
        LlmCognitionCellStatus::BlockedByProvider => "BLOCKED",
        LlmCognitionCellStatus::BlockedByBudget => "BUDGET",
        LlmCognitionCellStatus::StructuralBlocked => "STRUCT",
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn unix_now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_profile() -> LlmProviderProfile {
        LlmProviderProfile {
            id: "unit-local".to_string(),
            provider_class: LlmProviderClass::WeakLocal,
            model: "unit-model".to_string(),
            endpoint_configured: true,
            opt_in_enabled: true,
            advisory_only: true,
        }
    }

    fn blocked_profile() -> LlmProviderProfile {
        LlmProviderProfile {
            endpoint_configured: false,
            opt_in_enabled: false,
            ..ready_profile()
        }
    }

    #[test]
    fn sampled_matrix_is_small_and_governed() {
        let cases = sampled_llm_cognition_cases(DEFAULT_MAX_CASES);
        assert_eq!(cases.len(), DEFAULT_MAX_CASES);
        for case in cases {
            assert!(case.tags.contains(&"llm-cognition-matrix".to_string()));
            assert!(case
                .governance
                .capability_ids
                .contains(&"llm.cognition_advisory".to_string()));
            assert_eq!(
                case.governance.priority,
                Some(EvalPriority::P4ProviderModelAdvisory)
            );
            assert_eq!(
                case.governance.cost_class,
                Some(EvalCostClass::C4WithLlmCognition)
            );
        }
    }

    #[test]
    fn blocked_provider_does_not_pass_as_success() {
        let report = run_llm_cognition_matrix_with(
            "unit-llm",
            LlmCognitionBudget::default(),
            vec![blocked_profile()],
        );

        assert_eq!(report.summary.sampled_cases, DEFAULT_MAX_CASES);
        assert_eq!(report.summary.total_cells, DEFAULT_MAX_CASES);
        assert_eq!(report.summary.advisory_ready, 0);
        assert_eq!(report.summary.blocked_by_provider, DEFAULT_MAX_CASES);
    }

    #[test]
    fn ready_profile_is_advisory_only_and_structural_authoritative() {
        let report = run_llm_cognition_matrix_with(
            "unit-llm",
            LlmCognitionBudget::default(),
            vec![ready_profile()],
        );

        assert_eq!(report.summary.advisory_ready, DEFAULT_MAX_CASES);
        assert!(report.cells.iter().all(|cell| cell.advisory_only));
        assert!(report
            .cells
            .iter()
            .all(|cell| cell.structural_authority == "policy_verifier_gui_oracle_wins"));
    }

    #[test]
    fn request_budget_blocks_later_cells() {
        let budget = LlmCognitionBudget {
            max_requests: 1,
            ..LlmCognitionBudget::default()
        };
        let report = run_llm_cognition_matrix_with("unit-llm", budget, vec![ready_profile()]);

        assert_eq!(report.summary.total_cells, DEFAULT_MAX_CASES);
        assert_eq!(report.summary.advisory_ready, 1);
        assert_eq!(report.summary.blocked_by_budget, DEFAULT_MAX_CASES - 1);
    }

    #[test]
    fn provider_class_parser_accepts_bounded_profiles() {
        assert_eq!(
            LlmProviderClass::from_str("local-weak"),
            Some(LlmProviderClass::WeakLocal)
        );
        assert_eq!(
            LlmProviderClass::from_str("medium_local"),
            Some(LlmProviderClass::MediumLocal)
        );
        assert_eq!(
            LlmProviderClass::from_str("cloud-strong"),
            Some(LlmProviderClass::StrongCloud)
        );
        assert_eq!(LlmProviderClass::from_str("leaderboard"), None);
    }
}
