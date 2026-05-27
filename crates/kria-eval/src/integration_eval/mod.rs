//! Integration Eval Harness — Phase 2 of the KRIA eval strategy.
//!
//! # Purpose
//!
//! These evals verify that KRIA's **substrate execution path** (write_file +
//! execute_bash, no GUI, no daemon) produces semantically correct output for
//! representative programming tasks.
//!
//! They are CI-safe: no display server, no uinput daemon, no keystroke
//! injection. Only `sh`, `python3`, and optionally `rustc` are required.
//!
//! # Highest-value blind spot covered
//!
//! "KRIA claims Done but the output was never surfaced to the user."
//! These evals assert that the program actually ran and produced the expected
//! output — the same contract enforced by `GoalTreeResult::terminal_output`
//! and `ObservableOutputChecker` in the live agent path.
//!
//! # Structure
//!
//! - [`harness`] — `EvalHarness` owns a per-case temp directory.
//! - [`verifier`] — `ObservableOutputChecker` asserts semantic output contracts.
//! - [`runner`] — `IntegrationEvalRunner` orchestrates write → execute → verify.
//! - [`suites`] — 10 `IntegrationEvalCase` definitions covering canonical tasks.

pub mod fault_injection;
pub mod goal_tree_eval;
pub mod harness;
pub mod runner;
pub mod suites;
pub mod verifier;

pub use runner::{IntegrationEvalCase, IntegrationEvalResult, IntegrationEvalRunner};
pub use verifier::{checker_for, CheckResult, ObservableOutputChecker};

// ============================================================================
// Integration tests — run with: cargo test -p kria-eval --lib integration_eval
// ============================================================================

#[cfg(test)]
mod tests {
    use super::runner::IntegrationEvalRunner;
    use super::suites;

    fn runner() -> IntegrationEvalRunner {
        IntegrationEvalRunner { verbose: false }
    }

    #[tokio::test]
    async fn pascal_triangle() {
        let result = runner().run(&suites::pascal_triangle()).await;
        assert!(result.passed, "pascal_triangle failed: {}", result.reason);
    }

    #[tokio::test]
    async fn fibonacci() {
        let result = runner().run(&suites::fibonacci()).await;
        assert!(result.passed, "fibonacci failed: {}", result.reason);
    }

    #[tokio::test]
    async fn hello_world_python() {
        let result = runner().run(&suites::hello_world_python()).await;
        assert!(
            result.passed,
            "hello_world_python failed: {}",
            result.reason
        );
    }

    #[tokio::test]
    async fn write_and_verify() {
        let result = runner().run(&suites::write_and_verify()).await;
        assert!(result.passed, "write_and_verify failed: {}", result.reason);
    }

    #[tokio::test]
    async fn bubble_sort_python() {
        let result = runner().run(&suites::bubble_sort_python()).await;
        assert!(
            result.passed,
            "bubble_sort_python failed: {}",
            result.reason
        );
    }

    #[tokio::test]
    async fn run_bash_script() {
        let result = runner().run(&suites::run_bash_script()).await;
        assert!(result.passed, "run_bash_script failed: {}", result.reason);
    }

    #[tokio::test]
    async fn file_line_count() {
        let result = runner().run(&suites::file_line_count()).await;
        assert!(result.passed, "file_line_count failed: {}", result.reason);
    }

    #[tokio::test]
    async fn python_json_operations() {
        let result = runner().run(&suites::python_json_operations()).await;
        assert!(
            result.passed,
            "python_json_operations failed: {}",
            result.reason
        );
    }

    #[tokio::test]
    async fn matrix_multiply_python() {
        let result = runner().run(&suites::matrix_multiply_python()).await;
        assert!(
            result.passed,
            "matrix_multiply_python failed: {}",
            result.reason
        );
    }

    #[tokio::test]
    async fn hello_world_rust() {
        if which::which("rustc").is_err() {
            eprintln!("[SKIP] hello_world_rust: rustc not found");
            return;
        }
        let result = runner().run(&suites::hello_world_rust()).await;
        assert!(result.passed, "hello_world_rust failed: {}", result.reason);
    }

    #[tokio::test]
    async fn full_nodisplay_suite_all_pass() {
        let r = IntegrationEvalRunner { verbose: true };
        let cases = suites::all_nodisplay_cases();
        let results = r.run_suite(&cases).await;
        let failures: Vec<_> = results.iter().filter(|r| !r.passed).collect();
        if !failures.is_empty() {
            let msg = failures
                .iter()
                .map(|r| format!("  [FAIL] {}: {}", r.name, r.reason))
                .collect::<Vec<_>>()
                .join("\n");
            panic!("Integration eval suite failures:\n{}", msg);
        }
    }
}
