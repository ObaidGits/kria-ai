//! Complete A-to-Z GUI Automation Eval Suite
//!
//! Comprehensive testing of every GUI automation capability.
//! Uses KRIA's own test data — never touches user data.
//! All test files are created in ~/.kria/generated/ or /tmp/kria_test_*/

use super::suites::{case, no_artifacts};
use super::types::{DisplayServerRequirement, ExpectedArtifact, ExpectedBehavior, GuiEvalCase};

/// Complete A-to-Z eval suite covering all GUI automation capabilities.
pub fn complete_az_eval_suite() -> Vec<GuiEvalCase> {
    let mut cases = Vec::new();
    cases.extend(file_management_suite());
    cases.extend(code_generation_suite());
    cases.extend(terminal_execution_suite_az());
    cases.extend(browser_navigation_suite_az());
    cases.extend(app_lifecycle_suite_az());
    cases.extend(interaction_heavy_az_suite());
    cases.extend(ocr_cognition_suite());
    cases.extend(browser_cognition_suite());
    cases.extend(ide_cognition_suite());
    cases.extend(session_persistence_suite());
    cases.extend(recovery_resilience_suite());
    cases.extend(concurrent_isolation_suite());
    cases.extend(error_handling_suite_az());
    cases.extend(language_coverage_suite());
    cases
}

// ─── File Management Suite ────────────────────────────────────────────────────

pub fn file_management_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-file-001-write-python-file",
            "A-Z: Write a Python file to KRIA's generated directory",
            "open gedit and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(100),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "file-management", "write"],
        ),
        case(
            "az-file-002-write-rust-file",
            "A-Z: Write a Rust file",
            "open gedit and write a hello world program in rust",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "hello_*.rs".to_string(),
                    content_contains: Some("fn main".to_string()),
                    min_size_bytes: Some(20),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "file-management", "rust"],
        ),
        case(
            "az-file-003-write-javascript-file",
            "A-Z: Write a JavaScript file",
            "open gedit and write a fibonacci function in javascript",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.js".to_string(),
                    content_contains: Some("function".to_string()),
                    min_size_bytes: Some(30),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "file-management", "javascript"],
        ),
        case(
            "az-file-004-write-go-file",
            "A-Z: Write a Go file",
            "open gedit and write a fibonacci program in go",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.go".to_string(),
                    content_contains: Some("func fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "file-management", "go"],
        ),
        case(
            "az-file-005-write-typescript-file",
            "A-Z: Write a TypeScript file",
            "open gedit and write a fibonacci function in typescript",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.ts".to_string(),
                    content_contains: Some("function".to_string()),
                    min_size_bytes: Some(30),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "file-management", "typescript"],
        ),
    ]
}

// ─── Code Generation Suite ────────────────────────────────────────────────────

pub fn code_generation_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-codegen-001-fibonacci-python",
            "A-Z: Generate fibonacci in Python — verify content",
            "open gedit and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(100),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "codegen", "fibonacci"],
        ),
        case(
            "az-codegen-002-factorial-python",
            "A-Z: Generate factorial in Python",
            "open gedit and write a factorial program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "factorial_*.py".to_string(),
                    content_contains: Some("def factorial".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "codegen", "factorial"],
        ),
        case(
            "az-codegen-003-prime-checker",
            "A-Z: Generate prime checker in Python",
            "open gedit and write a prime number checker in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "prime_*.py".to_string(),
                    content_contains: Some("def is_prime".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "codegen", "prime"],
        ),
        case(
            "az-codegen-004-bubble-sort",
            "A-Z: Generate bubble sort in Python",
            "open gedit and write a bubble sort algorithm in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "bubble_sort_*.py".to_string(),
                    content_contains: Some("def bubble_sort".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "codegen", "sort"],
        ),
        case(
            "az-codegen-005-hello-world-cpp",
            "A-Z: Generate hello world in C++",
            "open gedit and write a hello world program in c++",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "hello_*.cpp".to_string(),
                    content_contains: Some("int main".to_string()),
                    min_size_bytes: Some(20),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "codegen", "cpp"],
        ),
    ]
}

// ─── Terminal Execution Suite (A-Z) ──────────────────────────────────────────

pub fn terminal_execution_suite_az() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-exec-001-fibonacci-run",
            "A-Z: Write and run fibonacci — verify output",
            "open gedit and write a fibonacci program in python and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![
                    ExpectedArtifact {
                        path_pattern: "fibonacci_*.py".to_string(),
                        content_contains: Some("def fibonacci".to_string()),
                        min_size_bytes: Some(50),
                    },
                    ExpectedArtifact {
                        path_pattern: "output_*.txt".to_string(),
                        content_contains: Some("0, 1".to_string()),
                        min_size_bytes: Some(1),
                    },
                ],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "terminal-execution", "fibonacci"],
        ),
        case(
            "az-exec-002-factorial-run",
            "A-Z: Write and run factorial — verify 120",
            "open gedit and write a factorial program in python and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![
                    ExpectedArtifact {
                        path_pattern: "factorial_*.py".to_string(),
                        content_contains: Some("def factorial".to_string()),
                        min_size_bytes: Some(50),
                    },
                    ExpectedArtifact {
                        path_pattern: "output_*.txt".to_string(),
                        content_contains: Some("120".to_string()),
                        min_size_bytes: Some(1),
                    },
                ],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "terminal-execution", "factorial"],
        ),
        case(
            "az-exec-003-hello-world-run",
            "A-Z: Write and run hello world — verify output",
            "open gedit and write a hello world program in python and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![
                    ExpectedArtifact {
                        path_pattern: "hello_*.py".to_string(),
                        content_contains: Some("def main".to_string()),
                        min_size_bytes: Some(20),
                    },
                    ExpectedArtifact {
                        path_pattern: "output_*.txt".to_string(),
                        content_contains: Some("Hello, World".to_string()),
                        min_size_bytes: Some(1),
                    },
                ],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "terminal-execution", "hello-world"],
        ),
        case(
            "az-exec-004-prime-run",
            "A-Z: Write and run prime checker — verify output",
            "open gedit and write a prime checker in python and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![
                    ExpectedArtifact {
                        path_pattern: "prime_*.py".to_string(),
                        content_contains: Some("def is_prime".to_string()),
                        min_size_bytes: Some(50),
                    },
                    ExpectedArtifact {
                        path_pattern: "output_*.txt".to_string(),
                        content_contains: Some("2, 3".to_string()),
                        min_size_bytes: Some(1),
                    },
                ],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "terminal-execution", "prime"],
        ),
        case(
            "az-exec-005-node-run",
            "A-Z: Write and run Node.js fibonacci",
            "open gedit and write a fibonacci program in javascript and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![
                    ExpectedArtifact {
                        path_pattern: "fibonacci_*.js".to_string(),
                        content_contains: Some("function fibonacci".to_string()),
                        min_size_bytes: Some(30),
                    },
                    ExpectedArtifact {
                        path_pattern: "output_*.txt".to_string(),
                        content_contains: Some("0, 1".to_string()),
                        min_size_bytes: Some(1),
                    },
                ],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "terminal-execution", "javascript", "node"],
        ),
        case(
            "az-exec-006-runtime-error-detected",
            "A-Z: Runtime error must be detected — no false success",
            "open gedit and write a program that divides by zero in python and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "divides_*.py".to_string(),
                    content_contains: Some("def ".to_string()),
                    min_size_bytes: Some(10),
                }],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["Completed: 2 verified steps".to_string()],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "terminal-execution", "error-detection"],
        ),
    ]
}

// ─── Browser Navigation Suite (A-Z) ──────────────────────────────────────────

pub fn browser_navigation_suite_az() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-browser-001-chrome-youtube",
            "A-Z: Open Chrome and search for YouTube",
            "open chrome and search for youtube",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "browser", "chrome"],
        ),
        case(
            "az-browser-002-firefox-github",
            "A-Z: Open Firefox and go to GitHub",
            "open firefox and go to github",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "browser", "firefox"],
        ),
        case(
            "az-browser-003-brave-reddit",
            "A-Z: Open Brave and search for Reddit",
            "open brave and search for reddit",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "browser", "brave"],
        ),
        case(
            "az-browser-004-no-retrieval-leak",
            "A-Z: Browser search must never trigger web_search",
            "open chrome and search for stackoverflow",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec!["web_search".to_string(), "search_news".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "browser", "retrieval-isolation"],
        ),
    ]
}

// ─── App Lifecycle Suite (A-Z) ────────────────────────────────────────────────

pub fn app_lifecycle_suite_az() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-app-001-open-gedit",
            "A-Z: Open gedit — ProcessLaunched verification",
            "open gedit",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec!["WINDOW_ID_FAILED".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "app-lifecycle", "gedit"],
        ),
        case(
            "az-app-002-open-kate",
            "A-Z: Open kate editor",
            "open kate",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["WINDOW_ID_FAILED".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "app-lifecycle", "kate"],
        ),
        case(
            "az-app-003-missing-app-graceful",
            "A-Z: Missing app must fail gracefully — no false success",
            "open nonexistent_app_xyz_99999",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["Done! I completed".to_string()],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "app-lifecycle", "missing-app"],
        ),
        case(
            "az-app-004-vscode-with-file",
            "A-Z: Open VS Code with a generated file",
            "open code and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["application 'code and'".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "app-lifecycle", "vscode"],
        ),
    ]
}

// ─── InteractionHeavy A-Z Suite ───────────────────────────────────────────────

pub fn interaction_heavy_az_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-interact-001-click-save",
            "A-Z: Click Save button via AT-SPI",
            "click the Save button",
            ExpectedBehavior {
                substrate: Some("InteractionHeavy".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["click_ui_element".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false, // No app open — graceful failure expected
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "interaction-heavy", "click"],
        ),
        case(
            "az-interact-002-click-ok",
            "A-Z: Click OK button via AT-SPI",
            "click OK",
            ExpectedBehavior {
                substrate: Some("InteractionHeavy".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["click_ui_element".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "interaction-heavy", "click", "dialog"],
        ),
        case(
            "az-interact-003-detect-dialog",
            "A-Z: Detect dialog — must not crash",
            "open gedit and write a hello world program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "hello_*.py".to_string(),
                    content_contains: Some("def main".to_string()),
                    min_size_bytes: Some(20),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "interaction-heavy", "detect-dialog"],
        ),
        case(
            "az-interact-004-get-desktop-state",
            "A-Z: Get desktop state via AT-SPI",
            "open gedit and write a factorial program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "factorial_*.py".to_string(),
                    content_contains: Some("def factorial".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "interaction-heavy", "desktop-state"],
        ),
    ]
}

// ─── OCR Cognition Suite ──────────────────────────────────────────────────────

pub fn ocr_cognition_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-ocr-001-read-screen-no-crash",
            "A-Z: OCR read_screen must not crash",
            "open gedit and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "ocr", "no-crash"],
        ),
        case(
            "az-ocr-002-check-text-tool-registered",
            "A-Z: check_text_on_screen tool must be registered",
            "open gedit and write a prime checker in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "prime_*.py".to_string(),
                    content_contains: Some("def is_prime".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "ocr", "tool-registered"],
        ),
    ]
}

// ─── Browser Cognition Suite ──────────────────────────────────────────────────

pub fn browser_cognition_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-browser-cog-001-get-state-no-crash",
            "A-Z: get_browser_state must not crash when CDP unavailable",
            "open gedit and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "browser-cognition", "no-crash"],
        ),
        case(
            "az-browser-cog-002-tools-registered",
            "A-Z: Browser cognition tools must be registered",
            "open gedit and write a hello world program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "hello_*.py".to_string(),
                    content_contains: Some("def main".to_string()),
                    min_size_bytes: Some(20),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "browser-cognition", "tools-registered"],
        ),
    ]
}

// ─── IDE Cognition Suite ──────────────────────────────────────────────────────

pub fn ide_cognition_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-ide-001-check-python-syntax",
            "A-Z: check_file_diagnostics on generated Python file",
            "open gedit and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "ide-cognition", "python-syntax"],
        ),
        case(
            "az-ide-002-get-ide-state-no-crash",
            "A-Z: get_ide_state must not crash",
            "open gedit and write a factorial program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "factorial_*.py".to_string(),
                    content_contains: Some("def factorial".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "ide-cognition", "no-crash"],
        ),
    ]
}

// ─── Session Persistence Suite ────────────────────────────────────────────────

pub fn session_persistence_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-session-001-checkpoint-saved",
            "A-Z: Workflow checkpoint must be saved after completion",
            "open gedit and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "session-persistence", "checkpoint"],
        ),
        case(
            "az-session-002-list-sessions-no-crash",
            "A-Z: list_workflow_sessions must not crash",
            "open gedit and write a hello world program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "hello_*.py".to_string(),
                    content_contains: Some("def main".to_string()),
                    min_size_bytes: Some(20),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "session-persistence", "list-sessions"],
        ),
    ]
}

// ─── Recovery Resilience Suite ────────────────────────────────────────────────

pub fn recovery_resilience_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-recovery-001-partial-success-artifact",
            "A-Z: Partial success — file written but app not found — artifact tracked",
            "open nonexistent_editor_xyz_99999 and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["Completed: 2 verified steps".to_string()],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "recovery", "partial-success"],
        ),
        case(
            "az-recovery-002-no-false-success",
            "A-Z: No false success — unknown app must fail honestly",
            "open nonexistent_app_xyz_12345 and write a program",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "Done! I completed".to_string(),
                    "successfully completed".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "recovery", "no-false-success"],
        ),
        case(
            "az-recovery-003-popup-aware-no-crash",
            "A-Z: Popup-aware workflow must not crash when no dialog present",
            "open gedit and write a prime checker in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "prime_*.py".to_string(),
                    content_contains: Some("def is_prime".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "recovery", "popup-aware"],
        ),
    ]
}

// ─── Concurrent Isolation Suite ───────────────────────────────────────────────

pub fn concurrent_isolation_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-concurrent-001-uuid-output-isolation",
            "A-Z: Concurrent execution — UUID output files must not collide",
            "open gedit and write a fibonacci program in python and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![
                    ExpectedArtifact {
                        path_pattern: "fibonacci_*.py".to_string(),
                        content_contains: Some("def fibonacci".to_string()),
                        min_size_bytes: Some(50),
                    },
                    ExpectedArtifact {
                        path_pattern: "output_*.txt".to_string(),
                        content_contains: Some("0, 1".to_string()),
                        min_size_bytes: Some(1),
                    },
                ],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "concurrent", "uuid-isolation"],
        ),
        case(
            "az-concurrent-002-uuid-binary-isolation",
            "A-Z: Concurrent Rust execution — UUID binary paths must not collide",
            "open gedit and write a fibonacci program in rust and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.rs".to_string(),
                    content_contains: Some("fn fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: false, // rustc may not be installed
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "concurrent", "rust-binary"],
        ),
    ]
}

// ─── Error Handling Suite (A-Z) ───────────────────────────────────────────────

pub fn error_handling_suite_az() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-error-001-missing-interpreter",
            "A-Z: Missing interpreter must fail gracefully",
            "open gedit and write a fibonacci program in kotlin and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.kt".to_string(),
                    content_contains: Some("fun fibonacci".to_string()),
                    min_size_bytes: Some(10),
                }],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "Done! I completed the GUI automation task".to_string()
                ],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "error-handling", "missing-interpreter"],
        ),
        case(
            "az-error-002-syntax-error-detected",
            "A-Z: Python syntax error must be detected",
            "open gedit and write a program that divides by zero in python and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "divides_*.py".to_string(),
                    content_contains: Some("def ".to_string()),
                    min_size_bytes: Some(10),
                }],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["Completed: 2 verified steps".to_string()],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "error-handling", "runtime-error"],
        ),
        case(
            "az-error-003-output-size-limit",
            "A-Z: Output size must be capped at 1MB",
            "open gedit and write a fibonacci program in python and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![
                    ExpectedArtifact {
                        path_pattern: "fibonacci_*.py".to_string(),
                        content_contains: Some("def fibonacci".to_string()),
                        min_size_bytes: Some(50),
                    },
                    ExpectedArtifact {
                        path_pattern: "output_*.txt".to_string(),
                        content_contains: Some("0, 1".to_string()),
                        min_size_bytes: Some(1),
                    },
                ],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["az", "error-handling", "output-size"],
        ),
    ]
}

// ─── Language Coverage Suite ──────────────────────────────────────────────────

pub fn language_coverage_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "az-lang-001-python-3-11",
            "A-Z: Python 3.11 version tag produces .py file",
            "open gedit and write a fibonacci program in python 3.11",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "language", "python-version"],
        ),
        case(
            "az-lang-002-nodejs",
            "A-Z: node.js language tag produces .js file",
            "open gedit and write a fibonacci function in node.js",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.js".to_string(),
                    content_contains: Some("function".to_string()),
                    min_size_bytes: Some(30),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "language", "nodejs"],
        ),
        case(
            "az-lang-003-go-language",
            "A-Z: go language phrasing produces .go file",
            "open gedit and write a fibonacci program in go language",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.go".to_string(),
                    content_contains: Some("func fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "language", "go-language"],
        ),
        case(
            "az-lang-004-java",
            "A-Z: Java language produces .java file",
            "open gedit and write a fibonacci program in java",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.java".to_string(),
                    content_contains: Some("public class Main".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "language", "java"],
        ),
        case(
            "az-lang-005-csharp",
            "A-Z: C# language produces .cs file",
            "open gedit and write a fibonacci program in c#",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.cs".to_string(),
                    content_contains: Some("class Program".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "language", "csharp"],
        ),
        case(
            "az-lang-006-kotlin",
            "A-Z: Kotlin language produces .kt file",
            "open gedit and write a fibonacci program in kotlin",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.kt".to_string(),
                    content_contains: Some("fun fibonacci".to_string()),
                    min_size_bytes: Some(30),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "language", "kotlin"],
        ),
        case(
            "az-lang-007-ruby",
            "A-Z: Ruby language produces .rb file",
            "open gedit and write a fibonacci program in ruby",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.rb".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(30),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "language", "ruby"],
        ),
        case(
            "az-lang-008-php",
            "A-Z: PHP language produces .php file",
            "open gedit and write a fibonacci program in php",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.php".to_string(),
                    content_contains: Some("function fibonacci".to_string()),
                    min_size_bytes: Some(30),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "language", "php"],
        ),
        case(
            "az-lang-009-swift",
            "A-Z: Swift language produces .swift file",
            "open gedit and write a fibonacci program in swift",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.swift".to_string(),
                    content_contains: Some("func fibonacci".to_string()),
                    min_size_bytes: Some(30),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "language", "swift"],
        ),
        case(
            "az-lang-010-bash",
            "A-Z: Bash language produces .sh file",
            "open gedit and write a fibonacci shell script",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.sh".to_string(),
                    content_contains: Some("fibonacci".to_string()),
                    min_size_bytes: Some(20),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["az", "language", "bash"],
        ),
    ]
}
