//! ObservableOutputChecker — verifies program output against a semantic contract.
//!
//! This is the "observable completion" layer for integration evals. Rather than
//! asserting exact output strings, it checks that semantically meaningful
//! fragments are present, matching how a human would evaluate the result.

/// Contract that a program's output must satisfy.
#[derive(Debug, Clone, Default)]
pub struct ObservableOutputChecker {
    /// All of these substrings must appear in stdout (case-insensitive by default).
    pub expected_fragments: Vec<String>,
    /// The command must exit with code 0.
    pub expected_exit_zero: bool,
    /// stdout must be non-empty.
    pub expected_stdout_non_empty: bool,
    /// Case-insensitive fragment matching (default true).
    pub case_insensitive: bool,
}

/// Result of an observable output check.
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub passed: bool,
    /// Human-readable explanation of pass/fail.
    pub reason: String,
}

impl ObservableOutputChecker {
    /// Builder: require exit code 0.
    pub fn require_exit_zero(mut self) -> Self {
        self.expected_exit_zero = true;
        self
    }

    /// Builder: require non-empty stdout.
    pub fn require_non_empty_stdout(mut self) -> Self {
        self.expected_stdout_non_empty = true;
        self
    }

    /// Builder: add a required output fragment.
    pub fn expect(mut self, fragment: impl Into<String>) -> Self {
        self.expected_fragments.push(fragment.into());
        self
    }

    /// Builder: enable/disable case-insensitive matching (default: true).
    pub fn case_insensitive(mut self, v: bool) -> Self {
        self.case_insensitive = v;
        self
    }

    /// Check the observed output against the contract.
    pub fn check(&self, stdout: &str, _stderr: &str, exit_code: i32) -> CheckResult {
        if self.expected_exit_zero && exit_code != 0 {
            return CheckResult {
                passed: false,
                reason: format!(
                    "expected exit code 0 but got {}. stderr: {}",
                    exit_code,
                    _stderr.trim()
                ),
            };
        }

        if self.expected_stdout_non_empty && stdout.trim().is_empty() {
            return CheckResult {
                passed: false,
                reason: "expected non-empty stdout but got nothing".into(),
            };
        }

        let haystack = if self.case_insensitive {
            stdout.to_lowercase()
        } else {
            stdout.to_string()
        };

        for fragment in &self.expected_fragments {
            let needle = if self.case_insensitive {
                fragment.to_lowercase()
            } else {
                fragment.clone()
            };
            if !haystack.contains(&needle) {
                return CheckResult {
                    passed: false,
                    reason: format!(
                        "expected '{}' in stdout but it was not found.\nActual stdout:\n{}",
                        fragment,
                        stdout.trim()
                    ),
                };
            }
        }

        CheckResult {
            passed: true,
            reason: "all observable output contracts satisfied".into(),
        }
    }
}

/// Convenience: build a simple checker that requires exit 0 + non-empty stdout +
/// all provided fragments.
pub fn checker_for(fragments: &[&str]) -> ObservableOutputChecker {
    let mut c = ObservableOutputChecker {
        case_insensitive: true,
        ..Default::default()
    }
    .require_exit_zero()
    .require_non_empty_stdout();
    for f in fragments {
        c = c.expect(*f);
    }
    c
}
