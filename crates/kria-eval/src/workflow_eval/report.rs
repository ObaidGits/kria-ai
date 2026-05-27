//! Workflow eval report builder.
//!
//! Produces a structured `WorkflowEvalReport` after a batch run, including:
//! - Per-dimension aggregate scores
//! - Category breakdowns
//! - False-success and silent-completion incident registers
//! - Failure diagnostic summaries
//! - Production cognition readiness assessment

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::failure_analysis::FailureDiagnostic;
use super::types::{EvalWorkflowCategory, WorkflowEvalVerdict, WorkflowVerdictKind};

// ─── Aggregate Scores ─────────────────────────────────────────────────────────

/// Aggregate success rates across all five dimensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionAggregates {
    pub tool_pass_rate: f32,
    pub workflow_pass_rate: f32,
    pub semantic_pass_rate: f32,
    pub observable_pass_rate: f32,
    pub collaborative_pass_rate: Option<f32>,
    pub overall_pass_rate: f32,
    pub average_quality_score: f32,
}

/// Breakdown by workflow category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategorySummary {
    pub category: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub average_quality_score: f32,
    pub common_failure_kinds: Vec<String>,
}

/// An incident of false success (KRIA claimed done with no evidence).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalseSuccessIncident {
    pub case_id: String,
    pub prompt_excerpt: String,
    pub claimed_success_phrase: String,
}

/// An incident of silent completion (KRIA ran silently without surfacing output).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SilentCompletionIncident {
    pub case_id: String,
    pub prompt_excerpt: String,
    pub trigger_pattern: String,
}

// ─── Production Readiness ─────────────────────────────────────────────────────

/// Production cognition readiness tiers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReadinessTier {
    /// ≥90% semantic + observable pass rate.
    ProductionReady,
    /// 70–89%: significant gaps, limited deployment.
    LimitedDeployment,
    /// 50–69%: fragile; major issues blocking real-world use.
    Fragile,
    /// <50%: not suitable for real-world workflow use.
    NotReady,
}

impl ReadinessTier {
    pub fn from_pass_rate(rate: f32) -> Self {
        if rate >= 0.90 {
            Self::ProductionReady
        } else if rate >= 0.70 {
            Self::LimitedDeployment
        } else if rate >= 0.50 {
            Self::Fragile
        } else {
            Self::NotReady
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ProductionReady => "PRODUCTION_READY",
            Self::LimitedDeployment => "LIMITED_DEPLOYMENT",
            Self::Fragile => "FRAGILE",
            Self::NotReady => "NOT_READY",
        }
    }
}

/// Remaining weak spots and recommended improvements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeakSpot {
    pub area: String,
    pub description: String,
    pub affected_case_count: usize,
    pub recommended_action: String,
}

// ─── Full Report ──────────────────────────────────────────────────────────────

/// Complete workflow eval run report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEvalReport {
    pub run_id: String,
    pub generated_at: String,
    pub total_cases: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub dimension_aggregates: DimensionAggregates,
    pub category_summaries: Vec<CategorySummary>,
    pub false_success_incidents: Vec<FalseSuccessIncident>,
    pub silent_completion_incidents: Vec<SilentCompletionIncident>,
    pub failure_diagnostics: Vec<FailureDiagnostic>,
    pub readiness_tier: ReadinessTier,
    pub readiness_rationale: String,
    pub weak_spots: Vec<WeakSpot>,
    pub expansion_roadmap: Vec<String>,
}

// ─── Report Builder ───────────────────────────────────────────────────────────

pub struct WorkflowEvalReportBuilder {
    run_id: String,
    verdicts: Vec<(WorkflowEvalVerdict, Option<EvalWorkflowCategory>, String)>,
    diagnostics: Vec<FailureDiagnostic>,
}

impl WorkflowEvalReportBuilder {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            verdicts: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn add_result(
        &mut self,
        verdict: WorkflowEvalVerdict,
        category: EvalWorkflowCategory,
        prompt: impl Into<String>,
        diagnostic: Option<FailureDiagnostic>,
    ) {
        self.verdicts.push((verdict, Some(category), prompt.into()));
        if let Some(d) = diagnostic {
            self.diagnostics.push(d);
        }
    }

    pub fn build(self) -> WorkflowEvalReport {
        let total = self.verdicts.len();
        let passed = self
            .verdicts
            .iter()
            .filter(|(v, _, _)| v.kind.is_passing())
            .count();
        let skipped = self
            .verdicts
            .iter()
            .filter(|(v, _, _)| v.kind == WorkflowVerdictKind::Skip)
            .count();
        let failed = total - passed - skipped;

        let dim = compute_dimension_aggregates(&self.verdicts);
        let category_summaries = compute_category_summaries(&self.verdicts);
        let false_success_incidents = collect_false_success(&self.verdicts);
        let silent_completion_incidents = collect_silent_completion(&self.verdicts);
        let weak_spots = identify_weak_spots(
            &dim,
            &category_summaries,
            &false_success_incidents,
            &silent_completion_incidents,
        );
        let readiness_tier = ReadinessTier::from_pass_rate(dim.overall_pass_rate);
        let readiness_rationale = build_readiness_rationale(&readiness_tier, &dim, &weak_spots);
        let expansion_roadmap = build_roadmap(&category_summaries, &weak_spots);

        WorkflowEvalReport {
            run_id: self.run_id,
            generated_at: chrono_now(),
            total_cases: total,
            passed,
            failed,
            skipped,
            dimension_aggregates: dim,
            category_summaries,
            false_success_incidents,
            silent_completion_incidents,
            failure_diagnostics: self.diagnostics,
            readiness_tier,
            readiness_rationale,
            weak_spots,
            expansion_roadmap,
        }
    }
}

// ─── Aggregation helpers ──────────────────────────────────────────────────────

fn compute_dimension_aggregates(
    verdicts: &[(WorkflowEvalVerdict, Option<EvalWorkflowCategory>, String)],
) -> DimensionAggregates {
    if verdicts.is_empty() {
        return DimensionAggregates {
            tool_pass_rate: 0.0,
            workflow_pass_rate: 0.0,
            semantic_pass_rate: 0.0,
            observable_pass_rate: 0.0,
            collaborative_pass_rate: None,
            overall_pass_rate: 0.0,
            average_quality_score: 0.0,
        };
    }

    let n = verdicts.len() as f32;
    let non_skip: Vec<_> = verdicts
        .iter()
        .filter(|(v, _, _)| v.kind != WorkflowVerdictKind::Skip)
        .collect();
    let ns = non_skip.len().max(1) as f32;

    let tool = non_skip
        .iter()
        .filter(|(v, _, _)| v.success_levels.tool_success)
        .count() as f32
        / ns;
    let wf = non_skip
        .iter()
        .filter(|(v, _, _)| v.success_levels.workflow_success)
        .count() as f32
        / ns;
    let sem = non_skip
        .iter()
        .filter(|(v, _, _)| v.success_levels.semantic_success)
        .count() as f32
        / ns;
    let obs = non_skip
        .iter()
        .filter(|(v, _, _)| v.success_levels.observable_success)
        .count() as f32
        / ns;
    let overall = verdicts
        .iter()
        .filter(|(v, _, _)| v.kind.is_passing())
        .count() as f32
        / n;
    let quality = verdicts
        .iter()
        .map(|(v, _, _)| v.quality_score)
        .sum::<f32>()
        / n;

    let collab_verdicts: Vec<_> = non_skip
        .iter()
        .filter_map(|(v, _, _)| v.success_levels.collaborative_success)
        .collect();
    let collaborative = if collab_verdicts.is_empty() {
        None
    } else {
        Some(collab_verdicts.iter().filter(|&&b| b).count() as f32 / collab_verdicts.len() as f32)
    };

    DimensionAggregates {
        tool_pass_rate: tool,
        workflow_pass_rate: wf,
        semantic_pass_rate: sem,
        observable_pass_rate: obs,
        collaborative_pass_rate: collaborative,
        overall_pass_rate: overall,
        average_quality_score: quality,
    }
}

fn compute_category_summaries(
    verdicts: &[(WorkflowEvalVerdict, Option<EvalWorkflowCategory>, String)],
) -> Vec<CategorySummary> {
    let mut by_cat: HashMap<String, Vec<&WorkflowEvalVerdict>> = HashMap::new();
    for (v, cat, _) in verdicts {
        let key = cat
            .map(|c| c.as_str().to_string())
            .unwrap_or_else(|| "unknown".into());
        by_cat.entry(key).or_default().push(v);
    }

    let mut summaries: Vec<CategorySummary> = by_cat
        .into_iter()
        .map(|(cat, vs)| {
            let total = vs.len();
            let passed = vs.iter().filter(|v| v.kind.is_passing()).count();
            let skipped = vs
                .iter()
                .filter(|v| v.kind == WorkflowVerdictKind::Skip)
                .count();
            let failed = total - passed - skipped;
            let avg_quality = vs.iter().map(|v| v.quality_score).sum::<f32>() / total.max(1) as f32;
            let mut failure_kinds: HashMap<String, usize> = HashMap::new();
            for v in &vs {
                if !v.kind.is_passing() {
                    *failure_kinds
                        .entry(v.kind.as_str().to_string())
                        .or_insert(0) += 1;
                }
            }
            let mut common: Vec<(String, usize)> = failure_kinds.into_iter().collect();
            common.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            CategorySummary {
                category: cat,
                total,
                passed,
                failed,
                skipped,
                average_quality_score: avg_quality,
                common_failure_kinds: common.into_iter().map(|(k, _)| k).collect(),
            }
        })
        .collect();

    summaries.sort_by(|a, b| a.category.cmp(&b.category));
    summaries
}

fn collect_false_success(
    verdicts: &[(WorkflowEvalVerdict, Option<EvalWorkflowCategory>, String)],
) -> Vec<FalseSuccessIncident> {
    verdicts
        .iter()
        .filter(|(v, _, _)| v.kind == WorkflowVerdictKind::FalseSuccess)
        .map(|(v, _, prompt)| FalseSuccessIncident {
            case_id: v.case_id.clone(),
            prompt_excerpt: prompt.chars().take(80).collect(),
            claimed_success_phrase: v.failure_reason.clone().unwrap_or_else(|| "unknown".into()),
        })
        .collect()
}

fn collect_silent_completion(
    verdicts: &[(WorkflowEvalVerdict, Option<EvalWorkflowCategory>, String)],
) -> Vec<SilentCompletionIncident> {
    verdicts
        .iter()
        .filter(|(v, _, _)| v.kind == WorkflowVerdictKind::SilentCompletion)
        .map(|(v, _, prompt)| SilentCompletionIncident {
            case_id: v.case_id.clone(),
            prompt_excerpt: prompt.chars().take(80).collect(),
            trigger_pattern: v.failure_reason.clone().unwrap_or_else(|| "unknown".into()),
        })
        .collect()
}

fn identify_weak_spots(
    dim: &DimensionAggregates,
    cats: &[CategorySummary],
    false_success: &[FalseSuccessIncident],
    silent: &[SilentCompletionIncident],
) -> Vec<WeakSpot> {
    let mut spots = Vec::new();

    if !false_success.is_empty() {
        spots.push(WeakSpot {
            area: "False Success Detection".into(),
            description: format!(
                "{} cases where KRIA claimed success with no verifiable evidence",
                false_success.len()
            ),
            affected_case_count: false_success.len(),
            recommended_action:
                "Gate all success responses on ObservableCompletionEngine.verify_visible()".into(),
        });
    }

    if !silent.is_empty() {
        spots.push(WeakSpot {
            area: "Silent Completion".into(),
            description: format!(
                "{} cases where KRIA completed without surfacing the result",
                silent.len()
            ),
            affected_case_count: silent.len(),
            recommended_action:
                "Require observable output signals before emitting completion response".into(),
        });
    }

    if dim.semantic_pass_rate < 0.80 {
        spots.push(WeakSpot {
            area: "Semantic Completion".into(),
            description: format!(
                "Semantic pass rate is {:.0}% — KRIA tools execute but goals not achieved",
                dim.semantic_pass_rate * 100.0
            ),
            affected_case_count: 0,
            recommended_action:
                "Review WorkflowExpectationEngine templates and observable_completion policies"
                    .into(),
        });
    }

    if dim.observable_pass_rate < 0.80 {
        spots.push(WeakSpot {
            area: "Observable Output".into(),
            description: format!(
                "Observable pass rate is {:.0}% — results not being surfaced to users",
                dim.observable_pass_rate * 100.0
            ),
            affected_case_count: 0,
            recommended_action:
                "Audit loop_engine StreamEvent::Text emission for all workflow categories".into(),
        });
    }

    for cat in cats {
        let pass_rate = if cat.total > 0 {
            cat.passed as f32 / cat.total as f32
        } else {
            1.0
        };
        if pass_rate < 0.60 && cat.total >= 3 {
            spots.push(WeakSpot {
                area: format!("{} workflows", cat.category),
                description: format!(
                    "{:.0}% pass rate in {} category ({}/{} passing)",
                    pass_rate * 100.0,
                    cat.category,
                    cat.passed,
                    cat.total
                ),
                affected_case_count: cat.failed,
                recommended_action: format!(
                    "Prioritize {} eval improvements. Common failures: {:?}",
                    cat.category, cat.common_failure_kinds
                ),
            });
        }
    }

    spots
}

fn build_readiness_rationale(
    tier: &ReadinessTier,
    dim: &DimensionAggregates,
    spots: &[WeakSpot],
) -> String {
    format!(
        "{}: overall {:.0}% pass, semantic {:.0}%, observable {:.0}%. {} weak spots identified.",
        tier.as_str(),
        dim.overall_pass_rate * 100.0,
        dim.semantic_pass_rate * 100.0,
        dim.observable_pass_rate * 100.0,
        spots.len()
    )
}

fn build_roadmap(_cats: &[CategorySummary], spots: &[WeakSpot]) -> Vec<String> {
    let mut items = Vec::new();

    items.push(
        "1. Fix false-success detection: gate success responses on ObservableCompletionEngine"
            .into(),
    );
    items.push("2. Add silent-completion guards in loop_engine for all workflow categories".into());
    items.push("3. Expand semantic contracts for browser and multi-app workflows".into());
    items.push(
        "4. Add interruption-recovery evals for daemon crash and focus theft scenarios".into(),
    );
    items.push(
        "5. Expand stress tests: multi-hour workflows, daemon failure loops, event storm".into(),
    );
    items.push("6. Add long-horizon workflow tests: browser → IDE → terminal pipelines".into());

    for spot in spots {
        items.push(format!("7+. {}: {}", spot.area, spot.recommended_action));
    }

    items
}

fn chrono_now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| format!("unix:{}", d.as_secs()))
        .unwrap_or_else(|_| "unknown".into())
}
