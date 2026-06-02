//! Structured GUI Eval Report Generation.
//!
//! Produces a production-grade architectural blueprint from eval results,
//! including root-cause analysis, priority-ranked improvements, and
//! per-category diagnostics.

use super::governance::{build_governance_report, EvalGovernanceMetadata, GovernanceReport};
use super::invariants::{evaluate_invariants, InvariantReport};
use super::observability::{
    classify_case_observability, empty_failure_bundle_summary, CaseObservability,
    FailureBundleSummary, FlakeClassification,
};
use super::types::{
    FailureCategory, GuiEvalCase, GuiEvalObservation, GuiEvalPreflight, GuiEvalVerdict,
    GuiEvalVerdictKind,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Report Structures ────────────────────────────────────────────────────────

/// Full GUI eval run report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiEvalReport {
    pub run_id: String,
    pub generated_at: String,
    pub environment: EnvironmentInfo,
    pub summary: RunSummary,
    pub governance: GovernanceReport,
    pub failure_bundles: FailureBundleSummary,
    pub category_breakdown: Vec<CategoryBreakdown>,
    pub case_results: Vec<CaseResult>,
    pub architectural_findings: Vec<ArchitecturalFinding>,
    pub priority_improvements: Vec<PriorityImprovement>,
    pub false_success_incidents: Vec<FalseSuccessIncident>,
    pub retrieval_leakage_incidents: Vec<RetrievalLeakageIncident>,
}

/// Environment information at time of eval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    pub display_server: String,
    pub gui_matrix_profile: Option<String>,
    pub xdotool_available: bool,
    pub wmctrl_available: bool,
    pub kria_eval_mode: bool,
    pub llm_base_url: String,
}

/// High-level summary of the eval run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub total_cases: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub environment_blocked: usize,
    pub invariant_violation_cases: usize,
    pub potential_flake_cases: usize,
    pub false_success_count: usize,
    pub retrieval_leakage_count: usize,
    pub pass_rate: f32,
    pub average_quality_score: f32,
    pub total_duration_ms: u64,
}

/// Breakdown by failure category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryBreakdown {
    pub category: String,
    pub count: usize,
    pub case_ids: Vec<String>,
    pub priority: u8,
}

/// Result for a single case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseResult {
    pub case_id: String,
    pub description: String,
    pub prompt: String,
    pub verdict: String,
    pub failure_category: Option<String>,
    pub quality_score: f32,
    pub explanation: String,
    pub evidence: Vec<String>,
    pub recommended_fix: Option<String>,
    pub substrate_used: Option<String>,
    pub tools_called: Vec<String>,
    pub retrieval_tools_called: Vec<String>,
    pub artifacts_found: usize,
    pub duration_ms: u64,
    pub preflight: GuiEvalPreflight,
    pub invariants: InvariantReport,
    pub observability: CaseObservability,
    pub governance: EvalGovernanceMetadata,
}

/// An architectural finding from the eval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitecturalFinding {
    pub severity: FindingSeverity,
    pub component: String,
    pub finding: String,
    pub evidence: Vec<String>,
    pub affected_cases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Critical,
    High,
    Medium,
    Low,
}

/// A priority-ranked improvement recommendation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityImprovement {
    pub rank: usize,
    pub category: String,
    pub title: String,
    pub description: String,
    pub affected_cases: Vec<String>,
    pub estimated_impact: String,
    pub implementation_hint: String,
}

/// A false-success incident.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalseSuccessIncident {
    pub case_id: String,
    pub prompt: String,
    pub kria_response: String,
    pub artifacts_found: bool,
    pub evidence: String,
}

/// A retrieval leakage incident.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalLeakageIncident {
    pub case_id: String,
    pub prompt: String,
    pub leaked_tools: Vec<String>,
    pub cloud_llm_invoked: bool,
    pub llm_retry_count: u32,
}

// ─── Report Builder ───────────────────────────────────────────────────────────

/// Builds a GuiEvalReport from eval results.
pub struct GuiEvalReportBuilder {
    run_id: String,
    results: Vec<(GuiEvalCase, GuiEvalObservation, GuiEvalVerdict)>,
    total_duration_ms: u64,
}

impl GuiEvalReportBuilder {
    pub fn new(run_id: String) -> Self {
        Self {
            run_id,
            results: Vec::new(),
            total_duration_ms: 0,
        }
    }

    pub fn add_result(
        &mut self,
        case: GuiEvalCase,
        obs: GuiEvalObservation,
        verdict: GuiEvalVerdict,
    ) {
        self.total_duration_ms += obs.timings.total_ms;
        self.results.push((case, obs, verdict));
    }

    pub fn build(self) -> GuiEvalReport {
        let environment = EnvironmentInfo {
            display_server: super::lifecycle::detect_display_server().to_string(),
            gui_matrix_profile: std::env::var("KRIA_EVAL_GUI_MATRIX_PROFILE")
                .ok()
                .or_else(|| std::env::var("KRIA_EVAL_GUI_PROFILE").ok()),
            xdotool_available: super::lifecycle::xdotool_available(),
            wmctrl_available: super::lifecycle::wmctrl_available(),
            kria_eval_mode: std::env::var("KRIA_EVAL_MODE").is_ok()
                || std::env::var("KRIA_EVAL_GUI").as_deref() == Ok("1"),
            llm_base_url: std::env::var("KRIA_EVAL_LLM_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080/v1".to_string()),
        };

        let cases: Vec<GuiEvalCase> = self
            .results
            .iter()
            .map(|(case, _, _)| case.clone())
            .collect();
        let governance = build_governance_report(&cases);
        let summary = self.build_summary();
        let category_breakdown = self.build_category_breakdown();
        let case_results = self.build_case_results();
        let architectural_findings = self.build_architectural_findings();
        let priority_improvements = self.build_priority_improvements();
        let false_success_incidents = self.build_false_success_incidents();
        let retrieval_leakage_incidents = self.build_retrieval_leakage_incidents();

        GuiEvalReport {
            run_id: self.run_id,
            generated_at: chrono_now(),
            environment,
            summary,
            governance,
            failure_bundles: empty_failure_bundle_summary(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../eval_reports/failure_bundles"
            )),
            category_breakdown,
            case_results,
            architectural_findings,
            priority_improvements,
            false_success_incidents,
            retrieval_leakage_incidents,
        }
    }

    fn build_summary(&self) -> RunSummary {
        let total = self.results.len();
        let passed = self
            .results
            .iter()
            .filter(|(_, _, v)| v.kind == GuiEvalVerdictKind::Pass)
            .count();
        let skipped = self
            .results
            .iter()
            .filter(|(_, _, v)| v.kind == GuiEvalVerdictKind::Skip)
            .count();
        let environment_blocked = self
            .results
            .iter()
            .filter(|(_, _, v)| v.kind == GuiEvalVerdictKind::EnvironmentBlocked)
            .count();
        let false_success = self
            .results
            .iter()
            .filter(|(_, _, v)| v.kind == GuiEvalVerdictKind::FalseSuccess)
            .count();
        let retrieval_leakage = self
            .results
            .iter()
            .filter(|(_, _, v)| v.kind == GuiEvalVerdictKind::RetrievalLeakage)
            .count();
        let invariant_violation_cases = self
            .results
            .iter()
            .filter(|(case, obs, _)| {
                evaluate_invariants(case, obs).has_release_blocking_violation()
            })
            .count();
        let potential_flake_cases = self
            .results
            .iter()
            .filter(|(case, obs, verdict)| {
                let invariants = evaluate_invariants(case, obs);
                matches!(
                    classify_case_observability(case, obs, verdict, &invariants)
                        .flake_classification,
                    FlakeClassification::PotentialRuntimeFlake
                        | FlakeClassification::PotentialModelVariance
                )
            })
            .count();
        let failed = total - passed - skipped - environment_blocked;

        let pass_rate = if total > 0 {
            passed as f32 / total as f32
        } else {
            0.0
        };

        let avg_quality = if total > 0 {
            self.results
                .iter()
                .map(|(_, _, v)| v.quality_score)
                .sum::<f32>()
                / total as f32
        } else {
            0.0
        };

        RunSummary {
            total_cases: total,
            passed,
            failed,
            skipped,
            environment_blocked,
            invariant_violation_cases,
            potential_flake_cases,
            false_success_count: false_success,
            retrieval_leakage_count: retrieval_leakage,
            pass_rate,
            average_quality_score: avg_quality,
            total_duration_ms: self.total_duration_ms,
        }
    }

    fn build_category_breakdown(&self) -> Vec<CategoryBreakdown> {
        let mut by_category: HashMap<String, Vec<String>> = HashMap::new();

        for (case, _, verdict) in &self.results {
            if let Some(cat) = &verdict.failure_category {
                by_category
                    .entry(cat.as_str().to_string())
                    .or_default()
                    .push(case.id.clone());
            }
        }

        let mut breakdown: Vec<CategoryBreakdown> = by_category
            .into_iter()
            .map(|(category, case_ids)| {
                let priority = category_priority(&category);
                CategoryBreakdown {
                    count: case_ids.len(),
                    case_ids,
                    priority,
                    category,
                }
            })
            .collect();

        breakdown.sort_by_key(|b| b.priority);
        breakdown
    }

    fn build_case_results(&self) -> Vec<CaseResult> {
        self.results
            .iter()
            .map(|(case, obs, verdict)| {
                let invariants = evaluate_invariants(case, obs);
                let observability = classify_case_observability(case, obs, verdict, &invariants);
                CaseResult {
                    case_id: case.id.clone(),
                    description: case.description.clone(),
                    prompt: case.prompt.clone(),
                    verdict: verdict.kind.as_str().to_string(),
                    failure_category: verdict
                        .failure_category
                        .as_ref()
                        .map(|c| c.as_str().to_string()),
                    quality_score: verdict.quality_score,
                    explanation: verdict.explanation.clone(),
                    evidence: verdict.evidence.clone(),
                    recommended_fix: verdict.recommended_fix.clone(),
                    substrate_used: obs.trace.substrate_selected.clone(),
                    tools_called: obs.trace.tools_called.clone(),
                    retrieval_tools_called: obs.trace.retrieval_tools_called.clone(),
                    artifacts_found: obs.artifacts_found.len(),
                    duration_ms: obs.timings.total_ms,
                    preflight: obs.preflight.clone(),
                    invariants,
                    observability,
                    governance: case.governance.clone(),
                }
            })
            .collect()
    }

    fn build_architectural_findings(&self) -> Vec<ArchitecturalFinding> {
        let mut findings = Vec::new();

        // Finding 1: False success detection
        let false_success_cases: Vec<String> = self
            .results
            .iter()
            .filter(|(_, _, v)| v.kind == GuiEvalVerdictKind::FalseSuccess)
            .map(|(c, _, _)| c.id.clone())
            .collect();
        if !false_success_cases.is_empty() {
            findings.push(ArchitecturalFinding {
                severity: FindingSeverity::Critical,
                component: "loop_engine/mod.rs + htn_executor.rs".to_string(),
                finding: format!(
                    "KRIA falsely reported success in {} case(s) without verifiable artifacts. \
                     The hardcoded 'Done!' success string was replaced but the underlying \
                     verification chain may still have gaps.",
                    false_success_cases.len()
                ),
                evidence: false_success_cases
                    .iter()
                    .map(|id| format!("Case {}: false success detected", id))
                    .collect(),
                affected_cases: false_success_cases,
            });
        }

        // Finding 2: Retrieval leakage
        let leakage_cases: Vec<String> = self
            .results
            .iter()
            .filter(|(_, _, v)| v.kind == GuiEvalVerdictKind::RetrievalLeakage)
            .map(|(c, _, _)| c.id.clone())
            .collect();
        if !leakage_cases.is_empty() {
            findings.push(ArchitecturalFinding {
                severity: FindingSeverity::High,
                component: "turn_memory.rs + loop_engine/mod.rs".to_string(),
                finding: format!(
                    "Retrieval tools (web_search/search_news) leaked into {} GUI workflow(s). \
                     The detect_satisfaction() function may not be marking GUI launches as \
                     satisfied, causing the ReAct loop to continue and call retrieval tools.",
                    leakage_cases.len()
                ),
                evidence: leakage_cases
                    .iter()
                    .map(|id| format!("Case {}: retrieval leakage detected", id))
                    .collect(),
                affected_cases: leakage_cases,
            });
        }

        // Finding 3: Window management failures
        let window_failures: Vec<String> = self
            .results
            .iter()
            .filter(|(_, _, v)| v.failure_category == Some(FailureCategory::WindowManagement))
            .map(|(c, _, _)| c.id.clone())
            .collect();
        if !window_failures.is_empty() {
            findings.push(ArchitecturalFinding {
                severity: FindingSeverity::High,
                component: "kria-uinput-daemon/src/main.rs + execution_verifier_impl.rs".to_string(),
                finding: "Window state verification fails on Wayland because kria-uinput-daemon \
                          uses xdotool getactivewindow which is X11-only. \
                          The FileWriteThenOpen substrate correctly avoids this by using \
                          VerificationType::None for Step 2, but AppOpenOnly still uses WindowState."
                    .to_string(),
                evidence: window_failures
                    .iter()
                    .map(|id| format!("Case {}: WINDOW_ID_FAILED", id))
                    .collect(),
                affected_cases: window_failures,
            });
        }

        // Finding 4: App resolution
        let resolution_failures: Vec<String> = self
            .results
            .iter()
            .filter(|(_, _, v)| v.failure_category == Some(FailureCategory::AppResolution))
            .map(|(c, _, _)| c.id.clone())
            .collect();
        if !resolution_failures.is_empty() {
            findings.push(ArchitecturalFinding {
                severity: FindingSeverity::High,
                component: "intent_compiler_llm.rs::RuleIntentCompiler::extract_app".to_string(),
                finding: "App name extraction consumed conjunction words ('and', 'then') as part \
                          of the app name. The conjunction-aware extractor with STOP_TOKENS was \
                          added but may not cover all cases."
                    .to_string(),
                evidence: resolution_failures
                    .iter()
                    .map(|id| format!("Case {}: app resolution failure", id))
                    .collect(),
                affected_cases: resolution_failures,
            });
        }

        // Finding 5: Session reuse gap
        findings.push(ArchitecturalFinding {
            severity: FindingSeverity::Medium,
            component: "gui_substrate_planner.rs + tools/app_lifecycle.rs".to_string(),
            finding: "KRIA does not check if an app is already running before launching it. \
                      The SubstratePlanner always emits open_application_with_file without \
                      consulting LiveEnvironmentGrounder.running_process_subset. \
                      This can cause duplicate windows and lost context."
                .to_string(),
            evidence: vec![
                "SubstratePlanner.plan() does not call is_process_running()".to_string(),
                "OpenApplication tool always calls gio launch without checking /proc".to_string(),
            ],
            affected_cases: vec![],
        });

        // Finding 6: Wayland window detection gap
        findings.push(ArchitecturalFinding {
            severity: FindingSeverity::Medium,
            component: "environment_grounder.rs + kria-uinput-daemon".to_string(),
            finding: "On pure Wayland, GroundingCapabilities.has_window_query=false and \
                      has_window_list=false. The LiveEnvironmentGrounder returns empty facts. \
                      The SubstratePlanner doesn't use these facts anyway, but the AppOpenOnly \
                      substrate's WindowState verification will always fail on Wayland."
                .to_string(),
            evidence: vec![
                "DisplayServerType::Wayland.supports_x11_queries() = false".to_string(),
                "GroundingCapabilities::probe() sets has_window_query=false on Wayland".to_string(),
            ],
            affected_cases: vec![],
        });

        findings
    }

    fn build_priority_improvements(&self) -> Vec<PriorityImprovement> {
        vec![
            PriorityImprovement {
                rank: 1,
                category: "false_success".to_string(),
                title: "Eliminate all false-success reporting paths".to_string(),
                description: "Every success claim must be backed by a verified artifact or \
                              explicit evidence. The 'Done! I completed' string was removed \
                              but verify the entire execution path has no remaining fake-success paths."
                    .to_string(),
                affected_cases: self.cases_by_category(FailureCategory::FalseSuccess),
                estimated_impact: "Eliminates the most critical user-trust failure".to_string(),
                implementation_hint: "Audit all StreamEvent::Done emissions in loop_engine/mod.rs. \
                                     Ensure WorkflowResult.success is only true when all steps \
                                     with non-None verification passed."
                    .to_string(),
            },
            PriorityImprovement {
                rank: 2,
                category: "retrieval_leakage".to_string(),
                title: "Complete retrieval isolation for GUI workflows".to_string(),
                description: "GUI launch workflows must never trigger web_search/search_news. \
                              The detect_satisfaction() fix and forced_is_gui_launch filter \
                              were added — verify they cover all paths."
                    .to_string(),
                affected_cases: self.cases_by_category(FailureCategory::RetrievalLeakage),
                estimated_impact: "Eliminates confusing multi-tool responses for simple GUI tasks".to_string(),
                implementation_hint: "Add integration test: after browser_search succeeds, \
                                     verify turn_memory.is_satisfied() returns true and \
                                     the loop terminates without calling web_search."
                    .to_string(),
            },
            PriorityImprovement {
                rank: 3,
                category: "session_reuse".to_string(),
                title: "Add running-app detection before launching".to_string(),
                description: "Before emitting open_application or open_application_with_file, \
                              check if the target app is already running via /proc scanning. \
                              If running, skip the launch step and proceed to file writing."
                    .to_string(),
                affected_cases: self.cases_by_category(FailureCategory::SessionReuse),
                estimated_impact: "Prevents duplicate windows and lost workspace context".to_string(),
                implementation_hint: "In SubstratePlanner.plan_file_write_then_open(), call \
                                     is_process_running(app_name_to_binary(app)) before \
                                     emitting the open_application_with_file step. \
                                     If already running, use open_url with file:// URI instead."
                    .to_string(),
            },
            PriorityImprovement {
                rank: 4,
                category: "window_management".to_string(),
                title: "Replace WindowState verification with Wayland-compatible alternatives".to_string(),
                description: "AppOpenOnly substrate uses WindowState verification which requires \
                              xdotool (X11-only). Replace with ProcessLaunched verification \
                              which polls /proc and works on both X11 and Wayland."
                    .to_string(),
                affected_cases: self.cases_by_category(FailureCategory::WindowManagement),
                estimated_impact: "Makes AppOpenOnly substrate work on Wayland".to_string(),
                implementation_hint: "In SubstratePlanner.plan_app_open(), change verify from \
                                     VerificationType::WindowState to VerificationType::None \
                                     (same as FileWriteThenOpen Step 2). The dispatcher returns \
                                     Err on actual launch failure, so verification is redundant."
                    .to_string(),
            },
            PriorityImprovement {
                rank: 5,
                category: "app_resolution".to_string(),
                title: "Expand app alias coverage and conjunction parsing".to_string(),
                description: "Add more app aliases (kate, mousepad, xed, etc.) and ensure \
                              the STOP_TOKENS list is comprehensive. Consider fuzzy matching \
                              against InstalledAppRegistry.name_aliases."
                    .to_string(),
                affected_cases: self.cases_by_category(FailureCategory::AppResolution),
                estimated_impact: "Handles more app names without falling through to generic extraction".to_string(),
                implementation_hint: "In extract_app(), after the named-app checks, try \
                                     InstalledAppRegistry.resolve_alias() with the extracted \
                                     token before returning it. This gives the registry a chance \
                                     to validate the name."
                    .to_string(),
            },
        ]
    }

    fn build_false_success_incidents(&self) -> Vec<FalseSuccessIncident> {
        self.results
            .iter()
            .filter(|(_, _, v)| v.kind == GuiEvalVerdictKind::FalseSuccess)
            .map(|(case, obs, verdict)| FalseSuccessIncident {
                case_id: case.id.clone(),
                prompt: case.prompt.clone(),
                kria_response: obs.trace.final_response.clone(),
                artifacts_found: !obs.artifacts_found.is_empty(),
                evidence: verdict.evidence.join("; "),
            })
            .collect()
    }

    fn build_retrieval_leakage_incidents(&self) -> Vec<RetrievalLeakageIncident> {
        self.results
            .iter()
            .filter(|(_, _, v)| v.kind == GuiEvalVerdictKind::RetrievalLeakage)
            .map(|(case, obs, _)| RetrievalLeakageIncident {
                case_id: case.id.clone(),
                prompt: case.prompt.clone(),
                leaked_tools: obs.trace.retrieval_tools_called.clone(),
                cloud_llm_invoked: obs.trace.cloud_llm_invoked,
                llm_retry_count: obs.trace.llm_retry_count,
            })
            .collect()
    }

    fn cases_by_category(&self, category: FailureCategory) -> Vec<String> {
        self.results
            .iter()
            .filter(|(_, _, v)| v.failure_category.as_ref() == Some(&category))
            .map(|(c, _, _)| c.id.clone())
            .collect()
    }
}

fn category_priority(category: &str) -> u8 {
    match category {
        "false_success" => 1,
        "retrieval_leakage" => 2,
        "cloud_llm_leakage" => 3,
        "app_resolution" => 4,
        "semantic_parsing" => 5,
        "substrate_planning" => 6,
        "app_lifecycle" => 7,
        "session_reuse" => 8,
        "workflow_execution" => 9,
        "verification_failure" => 10,
        "window_management" => 11,
        "missing_recovery" => 12,
        "invariant_violation" => 13,
        "environment_blocked" => 16,
        _ => 16,
    }
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{}", secs)
}

/// Print a human-readable summary to stdout.
pub fn print_report_summary(report: &GuiEvalReport) {
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║         KRIA GUI Automation Eval Report                      ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    println!("Run ID:     {}", report.run_id);
    println!("Generated:  {}", report.generated_at);
    println!("Display:    {}", report.environment.display_server);
    if let Some(profile) = &report.environment.gui_matrix_profile {
        println!("Matrix:     {}", profile);
    }
    println!();
    println!("── Summary ──────────────────────────────────────────────────────");
    println!("  Total:          {}", report.summary.total_cases);
    println!(
        "  PASS:           {} ({:.0}%)",
        report.summary.passed,
        report.summary.pass_rate * 100.0
    );
    println!("  FAIL:           {}", report.summary.failed);
    println!("  SKIP:           {}", report.summary.skipped);
    println!("  ENV BLOCKED:    {}", report.summary.environment_blocked);
    println!(
        "  Invariant Viol: {}",
        report.summary.invariant_violation_cases
    );
    println!(
        "  Potential Flake: {}",
        report.summary.potential_flake_cases
    );
    println!(
        "  False Success:  {} ⚠️",
        report.summary.false_success_count
    );
    println!(
        "  Retrieval Leak: {} ⚠️",
        report.summary.retrieval_leakage_count
    );
    println!(
        "  Avg Quality:    {:.2}",
        report.summary.average_quality_score
    );
    println!("  Duration:       {}ms", report.summary.total_duration_ms);
    println!();

    println!("── Governance ───────────────────────────────────────────────────");
    println!("  Capabilities:   {}", report.governance.capabilities.len());
    println!(
        "  Missing Meta:   {}",
        report.governance.entropy.missing_metadata_count
    );
    println!(
        "  Dedup Groups:   {}",
        report.governance.entropy.duplicate_group_count
    );
    println!(
        "  Entropy Score:  {:.3}",
        report.governance.entropy.entropy_score
    );
    println!(
        "  Failure Bundles: {} written, {} omitted",
        report.failure_bundles.written, report.failure_bundles.omitted
    );
    println!();

    if !report.category_breakdown.is_empty() {
        println!("── Failure Categories (by priority) ─────────────────────────────");
        for cat in &report.category_breakdown {
            println!(
                "  [{:2}] {:25} {} case(s): {:?}",
                cat.priority, cat.category, cat.count, cat.case_ids
            );
        }
        println!();
    }

    if !report.false_success_incidents.is_empty() {
        println!("── False Success Incidents ───────────────────────────────────────");
        for inc in &report.false_success_incidents {
            println!(
                "  ❌ [{}] \"{}\"",
                inc.case_id,
                &inc.prompt[..inc.prompt.len().min(60)]
            );
            println!(
                "     Response: \"{}\"",
                &inc.kria_response[..inc.kria_response.len().min(80)]
            );
            println!("     Artifacts found: {}", inc.artifacts_found);
        }
        println!();
    }

    if !report.retrieval_leakage_incidents.is_empty() {
        println!("── Retrieval Leakage Incidents ───────────────────────────────────");
        for inc in &report.retrieval_leakage_incidents {
            println!(
                "  ⚠️  [{}] \"{}\"",
                inc.case_id,
                &inc.prompt[..inc.prompt.len().min(60)]
            );
            println!("     Leaked tools: {:?}", inc.leaked_tools);
            if inc.cloud_llm_invoked {
                println!("     Cloud LLM: {} retries", inc.llm_retry_count);
            }
        }
        println!();
    }

    println!("── Priority Improvements ────────────────────────────────────────");
    for imp in &report.priority_improvements {
        println!("  #{} [{}] {}", imp.rank, imp.category, imp.title);
        println!("     Impact: {}", imp.estimated_impact);
    }
    println!();

    println!("── Case Results ─────────────────────────────────────────────────");
    for result in &report.case_results {
        let icon = match result.verdict.as_str() {
            "PASS" => "✅",
            "SKIP" => "⏭️",
            "ENVIRONMENT_BLOCKED" => "🚧",
            "FALSE_SUCCESS" => "🚨",
            "RETRIEVAL_LEAKAGE" => "⚠️",
            _ => "❌",
        };
        println!("  {} [{}] {}", icon, result.case_id, result.verdict);
        if result.verdict != "PASS" && result.verdict != "SKIP" {
            println!("     {}", result.explanation);
            if let Some(fix) = &result.recommended_fix {
                println!("     Fix: {}", &fix[..fix.len().min(120)]);
            }
        }
    }
    println!();
}
