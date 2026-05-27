use kria_eval::gui_eval::destructive_safety::{
    print_destructive_safety_report, run_destructive_safety_suite,
};
use kria_eval::gui_eval::expanded_gui_evals::{
    print_expanded_gui_eval_report, run_expanded_gui_eval_suite, write_expanded_gui_eval_markdown,
};
use kria_eval::gui_eval::gui_hardening::{
    print_gui_hardening_report, run_gui_hardening_suite, write_gui_hardening_markdown,
};
use kria_eval::gui_eval::hitl_timeline::{print_hitl_timeline_report, run_hitl_timeline_suite};
use kria_eval::gui_eval::judge::GuiEvalJudge;
use kria_eval::gui_eval::llm_cognition_matrix::{
    print_llm_cognition_matrix_report, run_llm_cognition_matrix,
};
use kria_eval::gui_eval::matrix::{
    display_critical_matrix_suite, supported_matrix_profiles, GuiEvalMatrixProfile,
};
use kria_eval::gui_eval::observability::write_failure_bundles;
use kria_eval::gui_eval::observability_score::{
    print_observability_score_report, score_latest_observability_reports,
    write_observability_markdown,
};
use kria_eval::gui_eval::production_gui_workflows::{
    print_production_gui_workflow_report, run_production_gui_workflow_suite,
    write_production_gui_workflow_markdown,
};
use kria_eval::gui_eval::readiness_summary::{
    build_latest_readiness_report, print_readiness_report, write_readiness_markdown,
};
use kria_eval::gui_eval::report::{print_report_summary, GuiEvalReportBuilder};
use kria_eval::gui_eval::runner::GuiEvalRunner;
use kria_eval::gui_eval::suites;
use kria_eval::gui_eval::workflow_fidelity::{
    print_workflow_fidelity_report, run_workflow_fidelity_suite, write_workflow_fidelity_markdown,
};
use kria_eval::report::{EvalCaseResult, EvalRunReport};
use kria_eval::runner::run_eval_case;
use kria_eval::suite::load_suite;

const PROMPT_FILES: [&str; 2] = ["TestPrompts.txt", "VMTestPrompts.txt"];

/// Parse CLI args into a simple mode enum.
#[derive(Debug, Clone, PartialEq)]
enum EvalMode {
    /// Run the original text-prompt eval suites (default).
    General,
    /// Run the GUI automation eval suites.
    Gui,
    /// Run deterministic HITL timeline evals.
    GuiHitl,
    /// Run sampled advisory with-LLM GUI cognition matrix.
    GuiLlm,
    /// Run VM-only destructive safety dry-run evals.
    GuiDestructive,
    /// Score observability/debuggability of latest eval reports.
    GuiObservability,
    /// Build production-readiness snapshot from latest GUI cognition eval reports.
    GuiReadiness,
    /// Run deterministic workflow-fidelity evals for semantic GUI workflow intelligence.
    GuiFidelity,
    /// Run production GUI workflow-fidelity evals across realistic prompt shapes.
    GuiProduction,
    /// Run Phase 10 hardening/audit gate for GUI workflow intelligence.
    GuiHardening,
    /// Run newly added expanded GUI evals only.
    GuiExpanded,
    /// Run real desktop GUI evals with an explicit live-host opt-in.
    GuiLive,
    /// Run the bounded full GUI cognition eval sequence, excluding general text-prompt evals.
    GuiFull,
    /// Run both suites.
    All,
}

fn parse_mode() -> EvalMode {
    let args: Vec<String> = std::env::args().collect();
    for arg in &args[1..] {
        match arg.as_str() {
            "--gui" => return EvalMode::Gui,
            "--gui-hitl" => return EvalMode::GuiHitl,
            "--gui-llm" => return EvalMode::GuiLlm,
            "--gui-destructive" => return EvalMode::GuiDestructive,
            "--gui-observability" => return EvalMode::GuiObservability,
            "--gui-readiness" => return EvalMode::GuiReadiness,
            "--gui-fidelity" => return EvalMode::GuiFidelity,
            "--gui-production" => return EvalMode::GuiProduction,
            "--gui-hardening" => return EvalMode::GuiHardening,
            "--gui-expanded-evals" => return EvalMode::GuiExpanded,
            "--gui-live" => return EvalMode::GuiLive,
            "--gui-full" => return EvalMode::GuiFull,
            "--all" => return EvalMode::All,
            "--general" => return EvalMode::General,
            _ => {}
        }
    }
    // Also check env var for CI pipelines
    match std::env::var("KRIA_EVAL_MODE_OVERRIDE")
        .unwrap_or_default()
        .as_str()
    {
        "gui" => EvalMode::Gui,
        "gui-hitl" | "gui_hitl" => EvalMode::GuiHitl,
        "gui-llm" | "gui_llm" => EvalMode::GuiLlm,
        "gui-destructive" | "gui_destructive" => EvalMode::GuiDestructive,
        "gui-observability" | "gui_observability" => EvalMode::GuiObservability,
        "gui-readiness" | "gui_readiness" => EvalMode::GuiReadiness,
        "gui-fidelity" | "gui_fidelity" => EvalMode::GuiFidelity,
        "gui-production" | "gui_production" => EvalMode::GuiProduction,
        "gui-hardening" | "gui_hardening" => EvalMode::GuiHardening,
        "gui-expanded-evals" | "gui_expanded_evals" => EvalMode::GuiExpanded,
        "gui-live" | "gui_live" => EvalMode::GuiLive,
        "gui-full" | "gui_full" => EvalMode::GuiFull,
        "all" => EvalMode::All,
        _ => EvalMode::General,
    }
}

/// Filter GUI eval cases by tag if KRIA_EVAL_GUI_TAG is set.
fn filter_gui_cases(
    cases: Vec<kria_eval::gui_eval::types::GuiEvalCase>,
) -> Vec<kria_eval::gui_eval::types::GuiEvalCase> {
    if let Some(profile_name) = gui_matrix_profile_env() {
        let Some(profile) = GuiEvalMatrixProfile::from_str(&profile_name) else {
            eprintln!(
                "Unknown KRIA_EVAL_GUI_MATRIX_PROFILE='{}'. Supported profiles: {}",
                profile_name,
                supported_matrix_profiles().join(", ")
            );
            return Vec::new();
        };
        return display_critical_matrix_suite(profile);
    }

    if let Ok(tag) = std::env::var("KRIA_EVAL_GUI_TAG") {
        if !tag.trim().is_empty() {
            return cases
                .into_iter()
                .filter(|c| c.tags.iter().any(|t| t == tag.trim()))
                .collect();
        }
    }
    // If KRIA_EVAL_GUI_CI_SAFE=1, skip desktop-requiring cases
    if std::env::var("KRIA_EVAL_GUI_CI_SAFE").as_deref() == Ok("1") {
        return cases.into_iter().filter(|c| !c.requires_desktop).collect();
    }
    cases
}

fn gui_matrix_profile_env() -> Option<String> {
    std::env::var("KRIA_EVAL_GUI_MATRIX_PROFILE")
        .ok()
        .or_else(|| std::env::var("KRIA_EVAL_GUI_PROFILE").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    let mode = parse_mode();

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║              KRIA Evaluation Harness                         ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!("Mode: {:?}", mode);
    println!();

    std::fs::create_dir_all("tests-logs/eval_reports").expect("Failed to create report directory");

    // ── GUI Automation Eval ───────────────────────────────────────────────
    if mode == EvalMode::Gui || mode == EvalMode::GuiFull || mode == EvalMode::All {
        run_gui_eval().await;
    }

    // ── HITL Timeline Eval ────────────────────────────────────────────────
    if mode == EvalMode::GuiHitl || mode == EvalMode::GuiFull || mode == EvalMode::All {
        run_hitl_timeline_eval();
    }

    // ── Advisory With-LLM Cognition Matrix ───────────────────────────────────
    if mode == EvalMode::GuiLlm || mode == EvalMode::GuiFull || mode == EvalMode::All {
        run_llm_cognition_matrix_eval();
    }

    // ── VM-Only Destructive Safety Eval ──────────────────────────────────────
    if mode == EvalMode::GuiDestructive || mode == EvalMode::GuiFull || mode == EvalMode::All {
        run_destructive_safety_eval();
    }

    // ── Observability / Replay Scope Score ───────────────────────────────────
    if mode == EvalMode::GuiObservability || mode == EvalMode::GuiFull || mode == EvalMode::All {
        run_observability_score_eval();
    }

    // ── Production Readiness Snapshot ───────────────────────────────────────
    if mode == EvalMode::GuiReadiness || mode == EvalMode::GuiFull || mode == EvalMode::All {
        run_gui_readiness_eval();
    }

    // ── Workflow Fidelity Eval ──────────────────────────────────────────────
    if mode == EvalMode::GuiFidelity || mode == EvalMode::GuiFull || mode == EvalMode::All {
        run_workflow_fidelity_eval();
    }

    // ── Production GUI Workflow Eval ───────────────────────────────────────
    if mode == EvalMode::GuiProduction || mode == EvalMode::GuiFull || mode == EvalMode::All {
        run_production_gui_workflow_eval();
    }

    // ── Phase 10 GUI Hardening Audit ──────────────────────────────────────
    if mode == EvalMode::GuiHardening || mode == EvalMode::GuiFull || mode == EvalMode::All {
        run_gui_hardening_eval();
    }

    // ── Expanded GUI Evals ────────────────────────────────────────────────
    if mode == EvalMode::GuiExpanded || mode == EvalMode::GuiFull || mode == EvalMode::All {
        run_expanded_gui_eval();
    }

    // ── Live Desktop GUI Eval ─────────────────────────────────────────────
    if mode == EvalMode::GuiLive || mode == EvalMode::All {
        run_gui_live_eval().await;
    }

    // ── General Text-Prompt Eval ──────────────────────────────────────────
    if mode == EvalMode::General || mode == EvalMode::All {
        run_general_eval().await;
    }
}

fn run_hitl_timeline_eval() {
    let run_id = format!(
        "hitl-timeline-run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let report = run_hitl_timeline_suite(run_id);
    print_hitl_timeline_report(&report);

    let json = serde_json::to_string_pretty(&report).expect("Failed to serialize HITL report");
    let report_path = "tests-logs/eval_reports/hitl_timeline_latest_run.json";
    std::fs::write(report_path, json).expect("Failed to write HITL timeline report");
    println!("📝 HITL timeline report saved to: {}", report_path);
    println!();
}

fn run_llm_cognition_matrix_eval() {
    let run_id = format!(
        "llm-cognition-run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let report = run_llm_cognition_matrix(run_id);
    print_llm_cognition_matrix_report(&report);

    let json =
        serde_json::to_string_pretty(&report).expect("Failed to serialize LLM cognition report");
    let report_path = "tests-logs/eval_reports/llm_cognition_latest_run.json";
    std::fs::write(report_path, json).expect("Failed to write LLM cognition report");
    println!("📝 LLM cognition matrix report saved to: {}", report_path);
    println!();
}

fn run_destructive_safety_eval() {
    let run_id = format!(
        "destructive-safety-run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let report = run_destructive_safety_suite(run_id);
    print_destructive_safety_report(&report);

    let json = serde_json::to_string_pretty(&report)
        .expect("Failed to serialize destructive safety report");
    let report_path = "tests-logs/eval_reports/destructive_safety_latest_run.json";
    std::fs::write(report_path, json).expect("Failed to write destructive safety report");
    println!("📝 Destructive safety report saved to: {}", report_path);
    println!();
}

fn run_observability_score_eval() {
    let report = score_latest_observability_reports();
    print_observability_score_report(&report);

    let json = serde_json::to_string_pretty(&report)
        .expect("Failed to serialize observability score report");
    let json_path = "tests-logs/eval_reports/observability_latest_run.json";
    std::fs::write(json_path, json).expect("Failed to write observability score report");
    let markdown_path = "tests-logs/eval_reports/observability_latest.md";
    write_observability_markdown(&report, markdown_path)
        .expect("Failed to write observability score markdown");
    println!("📝 Observability score report saved to: {}", json_path);
    println!(
        "📝 Observability score markdown saved to: {}",
        markdown_path
    );
    println!();
}

fn run_gui_readiness_eval() {
    let report = build_latest_readiness_report();
    print_readiness_report(&report);

    let json =
        serde_json::to_string_pretty(&report).expect("Failed to serialize GUI readiness report");
    let json_path = "tests-logs/eval_reports/gui_cognition_readiness_latest_run.json";
    std::fs::write(json_path, json).expect("Failed to write GUI readiness report");
    let markdown_path = "tests-logs/eval_reports/gui_cognition_readiness_latest.md";
    write_readiness_markdown(&report, markdown_path)
        .expect("Failed to write GUI readiness markdown");
    println!("📝 GUI cognition readiness report saved to: {}", json_path);
    println!(
        "📝 GUI cognition readiness markdown saved to: {}",
        markdown_path
    );
    println!();
}

fn run_workflow_fidelity_eval() {
    let run_id = format!(
        "workflow-fidelity-run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let report = run_workflow_fidelity_suite(run_id);
    print_workflow_fidelity_report(&report);

    let json = serde_json::to_string_pretty(&report)
        .expect("Failed to serialize workflow fidelity report");
    let json_path = "tests-logs/eval_reports/workflow_fidelity_latest_run.json";
    std::fs::write(json_path, json).expect("Failed to write workflow fidelity report");
    let markdown_path = "tests-logs/eval_reports/workflow_fidelity_latest.md";
    write_workflow_fidelity_markdown(&report, markdown_path)
        .expect("Failed to write workflow fidelity markdown");
    println!("Workflow fidelity report saved to: {}", json_path);
    println!("Workflow fidelity markdown saved to: {}", markdown_path);
    println!();
}

fn run_production_gui_workflow_eval() {
    let run_id = format!(
        "production-gui-workflow-run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let report = run_production_gui_workflow_suite(run_id);
    print_production_gui_workflow_report(&report);

    let json = serde_json::to_string_pretty(&report)
        .expect("Failed to serialize production GUI workflow report");
    let json_path = "tests-logs/eval_reports/production_gui_workflows_latest_run.json";
    std::fs::write(json_path, json).expect("Failed to write production GUI workflow report");
    let markdown_path = "tests-logs/eval_reports/production_gui_workflows_latest.md";
    write_production_gui_workflow_markdown(&report, markdown_path)
        .expect("Failed to write production GUI workflow markdown");
    println!("Production GUI workflow report saved to: {}", json_path);
    println!(
        "Production GUI workflow markdown saved to: {}",
        markdown_path
    );
    println!();
}

fn run_gui_hardening_eval() {
    let run_id = format!(
        "gui-hardening-run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let report = run_gui_hardening_suite(run_id);
    print_gui_hardening_report(&report);

    let json =
        serde_json::to_string_pretty(&report).expect("Failed to serialize GUI hardening report");
    let json_path = "tests-logs/eval_reports/gui_hardening_latest_run.json";
    std::fs::write(json_path, json).expect("Failed to write GUI hardening report");
    let markdown_path = "tests-logs/eval_reports/gui_hardening_latest.md";
    write_gui_hardening_markdown(&report, markdown_path)
        .expect("Failed to write GUI hardening markdown");
    println!("GUI hardening report saved to: {}", json_path);
    println!("GUI hardening markdown saved to: {}", markdown_path);
    println!();
}

fn run_expanded_gui_eval() {
    let run_id = format!(
        "expanded-gui-run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let report = run_expanded_gui_eval_suite(run_id);
    print_expanded_gui_eval_report(&report);

    let json = serde_json::to_string_pretty(&report)
        .expect("Failed to serialize expanded GUI eval report");
    let json_path = "tests-logs/eval_reports/expanded_gui_evals_latest_run.json";
    std::fs::write(json_path, json).expect("Failed to write expanded GUI eval report");
    let markdown_path = "tests-logs/eval_reports/expanded_gui_evals_latest.md";
    write_expanded_gui_eval_markdown(&report, markdown_path)
        .expect("Failed to write expanded GUI eval markdown");
    println!("Expanded GUI eval report saved to: {}", json_path);
    println!("Expanded GUI eval markdown saved to: {}", markdown_path);
    println!();
}

async fn run_gui_eval() {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  GUI Automation Eval");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let all_cases = suites::all_suites();
    let cases = filter_gui_cases(all_cases);

    println!("Loaded {} GUI eval cases", cases.len());
    if cases.is_empty() {
        println!("No cases to run (check KRIA_EVAL_GUI_TAG / KRIA_EVAL_GUI_CI_SAFE).");
        return;
    }

    let run_id = format!(
        "gui-run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );

    let runner = GuiEvalRunner::new();
    let judge = GuiEvalJudge;
    let mut builder = GuiEvalReportBuilder::new(run_id.clone());

    for case in cases {
        println!("  [{:40}] running…", &case.id);
        let obs = runner.run(&case).await;
        let verdict = judge.evaluate(&case, &obs);

        let icon = match verdict.kind {
            kria_eval::gui_eval::types::GuiEvalVerdictKind::Pass => "✅",
            kria_eval::gui_eval::types::GuiEvalVerdictKind::Skip => "⏭️",
            kria_eval::gui_eval::types::GuiEvalVerdictKind::EnvironmentBlocked => "🚧",
            kria_eval::gui_eval::types::GuiEvalVerdictKind::FalseSuccess => "🚨",
            kria_eval::gui_eval::types::GuiEvalVerdictKind::RetrievalLeakage => "⚠️",
            kria_eval::gui_eval::types::GuiEvalVerdictKind::Fail => "❌",
        };
        println!("  {} {:40} {}", icon, &case.id, verdict.kind.as_str());
        if !verdict.kind.is_passing() {
            println!("     {}", verdict.explanation);
        }

        builder.add_result(case, obs, verdict);
    }

    let mut report = builder.build();
    let failure_bundle_dir = "tests-logs/eval_reports/failure_bundles";
    match write_failure_bundles(&report, failure_bundle_dir) {
        Ok(summary) => {
            report.failure_bundles = summary;
        }
        Err(error) => {
            eprintln!("Warning: failed to write GUI failure bundles: {error}");
        }
    }
    print_report_summary(&report);

    // Write JSON report
    let json = serde_json::to_string_pretty(&report).expect("Failed to serialize GUI report");
    let report_path = "tests-logs/eval_reports/gui_latest_run.json";
    std::fs::write(report_path, json).expect("Failed to write GUI report");
    println!("📝 GUI eval report saved to: {}", report_path);
    println!();
}

async fn run_gui_live_eval() {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Live Desktop GUI Eval");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let mut cases: Vec<_> = filter_gui_cases(suites::all_suites())
        .into_iter()
        .filter(is_live_gui_case)
        .collect();

    if let Ok(limit) = std::env::var("KRIA_EVAL_GUI_LIVE_LIMIT") {
        if let Ok(limit) = limit.parse::<usize>() {
            cases.truncate(limit);
        }
    }

    println!("Loaded {} live GUI cases", cases.len());
    if cases.is_empty() {
        println!(
            "No live GUI cases selected. Use KRIA_EVAL_GUI_TAG to select a desktop-safe case."
        );
        return;
    }

    let run_id = format!(
        "gui-live-run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );

    let runner = GuiEvalRunner::new();
    let judge = GuiEvalJudge;
    let mut builder = GuiEvalReportBuilder::new(run_id.clone());

    for case in cases {
        println!("  [{:40}] running live...", &case.id);
        let obs = runner.run(&case).await;
        let verdict = judge.evaluate(&case, &obs);
        println!("  {:40} {}", &case.id, verdict.kind.as_str());
        if !verdict.kind.is_passing() {
            println!("     {}", verdict.explanation);
        }
        builder.add_result(case, obs, verdict);
    }

    let mut report = builder.build();
    let failure_bundle_dir = "tests-logs/eval_reports/live_failure_bundles";
    match write_failure_bundles(&report, failure_bundle_dir) {
        Ok(summary) => report.failure_bundles = summary,
        Err(error) => eprintln!("Warning: failed to write live GUI failure bundles: {error}"),
    }
    print_report_summary(&report);

    let json = serde_json::to_string_pretty(&report).expect("Failed to serialize live GUI report");
    let report_path = "tests-logs/eval_reports/gui_live_latest_run.json";
    std::fs::write(report_path, json).expect("Failed to write live GUI report");
    println!("Live GUI eval report saved to: {}", report_path);
    println!();
}

fn is_live_gui_case(case: &kria_eval::gui_eval::types::GuiEvalCase) -> bool {
    case.requires_desktop
        && !case.tags.iter().any(|tag| {
            matches!(
                tag.as_str(),
                "vm" | "vm-only" | "destructive" | "dangerous" | "host-mutating"
            )
        })
}

async fn run_general_eval() {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  General Text-Prompt Eval");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let _llm_process = match kria_eval::llm_fixture::LlmFixture::start().await {
        Ok(fixture) => fixture,
        Err(e) => {
            eprintln!("❌ LLM Backend Failure: {e}");
            eprintln!("Skipping general eval. Use --gui to run only GUI evals.");
            return;
        }
    };

    let mut cases = Vec::new();
    for prompt_file in PROMPT_FILES {
        match load_suite(prompt_file) {
            Ok(mut loaded) => {
                println!("Loaded {} cases from {}", loaded.len(), prompt_file);
                cases.append(&mut loaded);
            }
            Err(e) => {
                eprintln!("Warning: failed to load '{}': {}", prompt_file, e);
            }
        }
    }

    if cases.is_empty() {
        println!("No general eval cases found.");
        return;
    }

    let run_id = format!(
        "run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );

    let mut report = EvalRunReport {
        run_id,
        summary: serde_json::json!({}),
        case_results: Vec::new(),
        environment: serde_json::json!({ "prompt_files": PROMPT_FILES }),
    };

    for case in cases {
        println!("  Running: {}", case.id);
        let (obs, verdict) = run_eval_case(case).await;
        println!(
            "    Grade: {}  |  {}",
            verdict.judge_grade,
            verdict.reasons.join(" | ")
        );
        report.case_results.push(EvalCaseResult {
            observation: obs,
            verdict,
        });
    }

    let pass_count = report
        .case_results
        .iter()
        .filter(|r| r.verdict.judge_grade.eq_ignore_ascii_case("PASS"))
        .count();
    let fail_count = report.case_results.len().saturating_sub(pass_count);

    report.summary = serde_json::json!({
        "total": report.case_results.len(),
        "pass": pass_count,
        "fail": fail_count,
    });

    println!(
        "\nGeneral eval summary: total={}, pass={}, fail={}",
        report.case_results.len(),
        pass_count,
        fail_count
    );

    let json = serde_json::to_string_pretty(&report).expect("Failed to serialize report");
    let report_path = "tests-logs/eval_reports/latest_run.json";
    std::fs::write(report_path, json).expect("Failed to write report");
    println!("📝 General eval report saved to: {}", report_path);
}
