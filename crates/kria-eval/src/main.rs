use kria_eval::report::{EvalCaseResult, EvalRunReport};
use kria_eval::runner::run_eval_case;
use kria_eval::suite::load_suite;

const PROMPT_FILES: [&str; 2] = ["TestPrompts.txt", "VMTestPrompts.txt"];

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    let _llm_process = match kria_eval::llm_fixture::LlmFixture::start().await {
        Ok(fixture) => fixture,
        Err(e) => {
            eprintln!("❌ EVAL HARNESS ABORTED: LLM Backend Failure");
            eprintln!("Error: {e}");
            eprintln!("Troubleshooting:");
            eprintln!("1. Ensure your KRIA_EVAL_LLM_CMD in .env is correct.");
            eprintln!("2. Check if you have enough GPU VRAM available.");
            eprintln!("3. Try starting the LLM manually to verify it works.");
            std::process::exit(1);
        }
    };

    let mut cases = Vec::new();
    for prompt_file in PROMPT_FILES {
        let mut loaded = match load_suite(prompt_file) {
            Ok(cases) => cases,
            Err(error) => {
                eprintln!("Failed to load suite '{}': {}", prompt_file, error);
                std::process::exit(1);
            }
        };
        println!("Loaded {} evaluation cases from {}", loaded.len(), prompt_file);
        cases.append(&mut loaded);
    }

    let run_id = format!(
        "run-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    );

    let mut report = EvalRunReport {
        run_id,
        summary: serde_json::json!({}),
        case_results: Vec::new(),
        environment: serde_json::json!({
            "prompt_files": PROMPT_FILES,
        }),
    };

    for case in cases {
        println!("Running Case: {}", case.id);
        let (obs, verdict) = run_eval_case(case).await;
        println!("  -> Raw Events: {:?}", obs.events);
        println!("  Grade: {}", verdict.judge_grade);
        println!("  Reasons: {}", verdict.reasons.join(" | "));
        report.case_results.push(EvalCaseResult {
            observation: obs,
            verdict,
        });
    }

    let pass_count = report
        .case_results
        .iter()
        .filter(|result| result.verdict.judge_grade.eq_ignore_ascii_case("PASS"))
        .count();
    let fail_count = report.case_results.len().saturating_sub(pass_count);

    report.summary = serde_json::json!({
        "total": report.case_results.len(),
        "pass": pass_count,
        "fail": fail_count,
    });

    println!(
        "Summary: total={}, pass={}, fail={}",
        report.case_results.len(),
        pass_count,
        fail_count
    );

    std::fs::create_dir_all("tests-logs/eval_reports").expect("Failed to create report directory");
    let json =
        serde_json::to_string_pretty(&report).expect("Failed to serialize report");
    std::fs::write("tests-logs/eval_reports/latest_run.json", json)
        .expect("Failed to write report file");
    println!("\n📝 Full evaluation report saved to: tests-logs/eval_reports/latest_run.json");
}
