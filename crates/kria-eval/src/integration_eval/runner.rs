//! IntegrationEvalRunner — CI-safe substrate execution evaluator.
//!
//! # Design
//!
//! Each [`IntegrationEvalCase`] specifies:
//!   1. A source file to write (name + content).
//!   2. A shell command to execute in the temp dir.
//!   3. An [`ObservableOutputChecker`] that asserts semantic correctness of stdout.
//!
//! The runner uses `tokio::fs` + `tokio::process::Command` directly — no GUI,
//! no daemon, no keystroke injection — making it safe to run in any CI
//! environment with a standard POSIX shell and Python 3.
//!
//! # Relationship to GoalTree / SubstratePlanner
//!
//! This eval exercises the same *semantic contract* as the `execute_bash`
//! substrate path, but without the overhead of the full agent loop. When a
//! GoalTree `Verb::Run` stage with `execute_bash` succeeds, the output should
//! match what these integration tests verify independently.

use std::time::Instant;

use super::harness::EvalHarness;
use super::verifier::ObservableOutputChecker;

// ============================================================================
// IntegrationEvalCase
// ============================================================================

/// A single integration eval case.
#[derive(Debug, Clone)]
pub struct IntegrationEvalCase {
    /// Short human-readable name (snake_case, e.g. "fibonacci").
    pub name: String,
    /// File to write into the temp dir (e.g. "fib.py").
    /// Use `""` when no file needs to be written (pure command test).
    pub file_name: String,
    /// Content to write into `file_name`. Ignored when `file_name` is empty.
    pub file_content: String,
    /// Shell command to run inside the temp dir. Interpreted by `sh -c`.
    pub command: String,
    /// Contract that the command's output must satisfy.
    pub checker: ObservableOutputChecker,
    /// Maximum wall-clock seconds to allow the command to run.
    pub timeout_sec: u64,
}

// ============================================================================
// IntegrationEvalResult
// ============================================================================

/// Result of running one integration eval case.
#[derive(Debug, Clone)]
pub struct IntegrationEvalResult {
    pub name: String,
    pub passed: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// Human-readable verdict from the checker.
    pub reason: String,
    pub duration_ms: u128,
}

impl IntegrationEvalResult {
    /// Print a one-line summary to stderr (useful in test output).
    pub fn print_summary(&self) {
        if self.passed {
            eprintln!("[PASS] {} ({}ms)", self.name, self.duration_ms);
        } else {
            eprintln!(
                "[FAIL] {} ({}ms): {}",
                self.name, self.duration_ms, self.reason
            );
        }
    }
}

// ============================================================================
// IntegrationEvalRunner
// ============================================================================

/// Runs integration eval cases in isolation using real OS process execution.
///
/// No GUI, no daemon, no sidecar required.
pub struct IntegrationEvalRunner {
    /// When true, print per-case summaries to stderr.
    pub verbose: bool,
}

impl Default for IntegrationEvalRunner {
    fn default() -> Self {
        Self { verbose: true }
    }
}

impl IntegrationEvalRunner {
    /// Run a single eval case in a fresh temp dir.
    pub async fn run(&self, case: &IntegrationEvalCase) -> IntegrationEvalResult {
        // ── Setup: allocate temp harness ──────────────────────────────────────
        let harness = match EvalHarness::new() {
            Ok(h) => h,
            Err(e) => {
                return IntegrationEvalResult {
                    name: case.name.clone(),
                    passed: false,
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: -1,
                    reason: format!("failed to create temp dir: {e}"),
                    duration_ms: 0,
                };
            }
        };

        // ── Write source file if specified ───────────────────────────────────
        if !case.file_name.is_empty() {
            if let Err(e) = harness.write_sync(&case.file_name, &case.file_content) {
                return IntegrationEvalResult {
                    name: case.name.clone(),
                    passed: false,
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: -1,
                    reason: format!("failed to write '{}': {e}", case.file_name),
                    duration_ms: 0,
                };
            }
        }

        // ── Execute command with timeout ──────────────────────────────────────
        let start = Instant::now();
        let timeout = tokio::time::Duration::from_secs(case.timeout_sec.max(1));

        let child_fut = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&case.command)
            .current_dir(harness.path())
            .output();

        let output_result = tokio::time::timeout(timeout, child_fut).await;
        let duration_ms = start.elapsed().as_millis();

        let (stdout, stderr, exit_code) = match output_result {
            Ok(Ok(output)) => (
                String::from_utf8_lossy(&output.stdout).into_owned(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
                output.status.code().unwrap_or(-1),
            ),
            Ok(Err(e)) => {
                let result = IntegrationEvalResult {
                    name: case.name.clone(),
                    passed: false,
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: -1,
                    reason: format!("failed to spawn command: {e}"),
                    duration_ms,
                };
                if self.verbose {
                    result.print_summary();
                }
                return result;
            }
            Err(_elapsed) => {
                let result = IntegrationEvalResult {
                    name: case.name.clone(),
                    passed: false,
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: -1,
                    reason: format!("command timed out after {}s", case.timeout_sec),
                    duration_ms,
                };
                if self.verbose {
                    result.print_summary();
                }
                return result;
            }
        };

        // ── Verify output against contract ────────────────────────────────────
        let check = case.checker.check(&stdout, &stderr, exit_code);
        let result = IntegrationEvalResult {
            name: case.name.clone(),
            passed: check.passed,
            stdout,
            stderr,
            exit_code,
            reason: check.reason,
            duration_ms,
        };

        if self.verbose {
            result.print_summary();
        }
        result
    }

    /// Run all cases sequentially and return all results.
    pub async fn run_suite(&self, cases: &[IntegrationEvalCase]) -> Vec<IntegrationEvalResult> {
        let mut results = Vec::with_capacity(cases.len());
        for case in cases {
            results.push(self.run(case).await);
        }
        results
    }

    /// Run all cases and return `(passed, failed)` counts.
    pub async fn run_suite_counts(&self, cases: &[IntegrationEvalCase]) -> (usize, usize) {
        let results = self.run_suite(cases).await;
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = results.len() - passed;
        (passed, failed)
    }
}
