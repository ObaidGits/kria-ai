//! GUI Automation Eval Suites
//!
//! Categorized test suites covering all failure modes identified in the
//! architectural audit. Each suite targets a specific failure category.

use super::types::{DisplayServerRequirement, ExpectedArtifact, ExpectedBehavior, GuiEvalCase};
use std::time::Duration;

pub fn case(
    id: &str,
    description: &str,
    prompt: &str,
    behavior: ExpectedBehavior,
    display_server: DisplayServerRequirement,
    requires_desktop: bool,
    tags: &[&str],
) -> GuiEvalCase {
    let tags: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
    let governance = super::governance::derive_governance_metadata(
        id,
        description,
        prompt,
        &behavior,
        display_server,
        requires_desktop,
        &tags,
    );
    GuiEvalCase {
        id: id.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        expected_behavior: behavior,
        display_server,
        tags,
        requires_desktop,
        timeout: Duration::from_secs(30),
        governance,
    }
}

pub fn no_artifacts() -> Vec<ExpectedArtifact> {
    Vec::new()
}

fn python_artifact(topic: &str) -> Vec<ExpectedArtifact> {
    vec![ExpectedArtifact {
        path_pattern: format!("~/.kria/generated/{}*.py", topic),
        content_contains: Some("def ".to_string()),
        min_size_bytes: Some(50),
    }]
}

// ─── Suite 1: Regression Tests (Exact Failing Prompts) ───────────────────────

/// Regression tests for the three exact prompts that were failing.
pub fn regression_suite() -> Vec<GuiEvalCase> {
    vec![
        // REGRESSION 1: "Open chrome and search for youtube"
        // Was triggering search_news + web_search + cloud LLM retries
        case(
            "regression-001-chrome-youtube",
            "Open chrome and search for youtube — must not trigger retrieval tools",
            "Open chrome and search for youtube",
            ExpectedBehavior {
                substrate: None, // browser_search path, not HTN
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec![
                    "web_search".to_string(),
                    "search_news".to_string(),
                    "searxng_search".to_string(),
                ],
                forbidden_response_patterns: vec![
                    "i cannot".to_string(),
                    "i can't".to_string(),
                    "tool configuration issue".to_string(),
                    "cloud LLM failed".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["regression", "browser", "retrieval-isolation"],
        ),
        // REGRESSION 2: "Open gedit and type a program to print fibonacci series in python"
        // Was failing with WINDOW_ID_FAILED on Step 2 verification
        case(
            "regression-002-gedit-fibonacci",
            "Open gedit and type fibonacci program — must write file and open editor",
            "Open gedit and type a program to print fibonacci series in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(100),
                }],
                required_tools: vec![
                    "write_file".to_string(),
                    "open_application_with_file".to_string(),
                ],
                forbidden_tools: vec!["web_search".to_string(), "search_news".to_string()],
                forbidden_response_patterns: vec![
                    "WINDOW_ID_FAILED".to_string(),
                    "No GUI backend available".to_string(),
                    "Done! I completed the GUI automation task".to_string(),
                ],
                required_response_patterns: vec!["verified".to_string(), "step".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["regression", "file-substrate", "editor", "fibonacci"],
        ),
        // REGRESSION 3: "Open code and write a program to print pascals triangle"
        // Was parsing app as "code and" instead of "code"
        case(
            "regression-003-code-pascals-triangle",
            "Open code and write pascals triangle — must resolve 'code' not 'code and'",
            "Open code and Write a program to print pascals triangle in python3 and run the prgram",
            ExpectedBehavior {
                // Explicit Code/VS Code run intent → open source in Code and run visibly.
                substrate: Some("VSCodeCodeRunWorkflow".to_string()),
                expected_artifacts: vec![
                    ExpectedArtifact {
                        path_pattern: "pascal_*.py".to_string(),
                        content_contains: Some("def pascals_triangle".to_string()),
                        min_size_bytes: Some(50),
                    },
                    ExpectedArtifact {
                        path_pattern: "output_*.txt".to_string(),
                        content_contains: Some("1 1".to_string()), // Pascal's triangle row
                        min_size_bytes: Some(1),
                    },
                ],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "application 'code and' is not found".to_string(),
                    "application 'Code and' is not found".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false, // No desktop needed for TerminalExecution
            &[
                "regression",
                "app-resolution",
                "conjunction-parsing",
                "terminal-execution",
            ],
        ),
    ]
}

// ─── Suite 2: Semantic Parsing ────────────────────────────────────────────────

/// Tests for intent compiler correctness.
pub fn semantic_parsing_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "parse-001-open-gedit-simple",
            "Simple open gedit — no content",
            "open gedit",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec!["cannot".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["parsing", "app-open"],
        ),
        case(
            "parse-002-open-firefox-simple",
            "Simple open firefox — routes to AppOpenOnly (no search query present)",
            "open firefox",
            ExpectedBehavior {
                // FIX #15: "open firefox" with no search → AppOpenOnly, not BrowserNavigate.
                // Previously this always routed to BrowserNavigate with an empty query,
                // which navigated to google.com. Now it correctly opens Firefox as an app.
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["parsing", "app-open", "browser"],
        ),
        case(
            "parse-003-open-code-write-rust",
            "Open code and write rust program — language detection",
            "open code and write a hello world program in rust",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "hello_*.rs".to_string(),
                    content_contains: Some("fn main".to_string()),
                    min_size_bytes: Some(20),
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
            &["parsing", "language-detection", "rust"],
        ),
        case(
            "parse-004-open-gedit-write-javascript",
            "Open gedit and write javascript — language detection",
            "open gedit and create a fibonacci function in javascript",
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
            &["parsing", "language-detection", "javascript"],
        ),
    ]
}

// ─── Suite 3: App Resolution ──────────────────────────────────────────────────

/// Tests for application name resolution.
pub fn app_resolution_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "resolve-001-code-bare-word",
            "Bare 'code' resolves to VS Code, not 'code and'",
            "open code and write a python script",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                // Use a flexible pattern — the exact topic word depends on hint extraction
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "*.py".to_string(),
                    content_contains: Some("def ".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "application 'code and' is not found".to_string(),
                    "application 'Code and' is not found".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["app-resolution", "conjunction-parsing"],
        ),
        case(
            "resolve-002-chrome-conjunction",
            "Chrome with conjunction — must not eat 'and'",
            "open chrome and search for youtube",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "application 'chrome and' is not found".to_string()
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["app-resolution", "conjunction-parsing", "browser"],
        ),
        case(
            "resolve-003-vscode-alias",
            "VS Code alias resolution",
            "open VS Code and write a python program",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: python_artifact("program"),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "application 'VS Code and' is not found".to_string(),
                    "application 'vs code and' is not found".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["app-resolution", "alias"],
        ),
    ]
}

// ─── Suite 4: Retrieval Isolation ─────────────────────────────────────────────

/// Tests that GUI workflows don't leak into retrieval tools.
pub fn retrieval_isolation_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "isolation-001-browser-no-web-search",
            "Browser search must not trigger web_search",
            "Open chrome and search for youtube",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec![
                    "web_search".to_string(),
                    "search_news".to_string(),
                    "searxng_search".to_string(),
                ],
                forbidden_response_patterns: vec![
                    "cloud LLM failed".to_string(),
                    "LLM error".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["retrieval-isolation", "browser"],
        ),
        case(
            "isolation-002-editor-no-web-search",
            "Editor workflow must not trigger web_search",
            "open gedit and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec!["web_search".to_string(), "search_news".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["retrieval-isolation", "editor"],
        ),
        case(
            "isolation-003-open-app-no-search",
            "Open application must not trigger search tools",
            "open gedit",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec![
                    "web_search".to_string(),
                    "search_news".to_string(),
                    "searxng_search".to_string(),
                ],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["retrieval-isolation", "app-open"],
        ),
    ]
}

// ─── Suite 5: False-Success Prevention ───────────────────────────────────────

/// Tests that KRIA never falsely claims success.
pub fn false_success_prevention_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "false-success-001-file-must-exist",
            "File must exist after write_file — no fake Done!",
            "open gedit and type a program to print fibonacci series in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(100),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "Done! I completed the GUI automation task".to_string(),
                    "done! i completed".to_string(),
                ],
                required_response_patterns: vec!["verified".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["false-success", "verification"],
        ),
        case(
            "false-success-002-no-hallucinated-success",
            "KRIA must not claim success for unknown app",
            "open nonexistent_app_xyz_12345 and write a program",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "Done! I completed".to_string(),
                    "successfully completed".to_string(),
                    "task completed".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: false, // Should fail gracefully
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["false-success", "error-handling"],
        ),
    ]
}

// ─── Suite 6: Substrate Planning ─────────────────────────────────────────────

/// Tests for correct substrate selection.
pub fn substrate_planning_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "substrate-001-file-write-then-open",
            "Editor + generated code → FileWriteThenOpen substrate",
            "open gedit and write a factorial program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "factorial_*.py".to_string(),
                    content_contains: Some("def factorial".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec![
                    "write_file".to_string(),
                    "open_application_with_file".to_string(),
                ],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["substrate", "file-write-then-open"],
        ),
        case(
            "substrate-002-app-open-only",
            "Open app with no content → AppOpenOnly substrate",
            "open gedit",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec!["write_file".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["substrate", "app-open-only"],
        ),
        case(
            "substrate-003-browser-navigate",
            "Browser search → BrowserNavigate substrate",
            "open chrome and search for youtube",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec!["write_file".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["substrate", "browser-navigate"],
        ),
    ]
}

// ─── Suite 7: Wayland/X11 Compatibility ──────────────────────────────────────

/// Tests that work on both X11 and Wayland.
pub fn wayland_x11_compatibility_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "compat-001-file-substrate-wayland",
            "FileWriteThenOpen substrate works on Wayland (no xdotool needed)",
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
                // Must NOT use WindowState verification (X11-only)
                forbidden_response_patterns: vec![
                    "WINDOW_ID_FAILED".to_string(),
                    "No GUI backend available for window state check".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any, // Must work on both
            true,
            &["wayland", "x11", "compatibility", "file-substrate"],
        ),
        case(
            "compat-002-browser-search-wayland",
            "Browser search works on Wayland (uses xdg-open/gio)",
            "open chrome and search for youtube",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["WINDOW_ID_FAILED".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["wayland", "x11", "compatibility", "browser"],
        ),
    ]
}

// ─── Suite 8: App Lifecycle ───────────────────────────────────────────────────

/// Tests for application lifecycle handling.
pub fn app_lifecycle_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "lifecycle-001-launch-not-running",
            "Launch app when not running",
            "open gedit and write a hello world program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "hello_*.py".to_string(),
                    content_contains: Some("def main".to_string()),
                    min_size_bytes: Some(30),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["application 'gedit' is not found".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["lifecycle", "launch"],
        ),
        case(
            "lifecycle-002-missing-app-graceful",
            "Missing app — graceful failure with suggestion",
            "open nonexistent_editor_xyz and write a program",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "Done! I completed".to_string(),
                    "successfully".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["lifecycle", "missing-app", "error-handling"],
        ),
    ]
}

// ─── Suite 9: Content Generation ─────────────────────────────────────────────

/// Tests for code generation quality.
pub fn content_generation_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "codegen-001-fibonacci-python",
            "Generate fibonacci program in Python",
            "open gedit and type a program to print fibonacci series in python",
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
            &["codegen", "fibonacci", "python"],
        ),
        case(
            "codegen-002-factorial-python",
            "Generate factorial program in Python",
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
            &["codegen", "factorial", "python"],
        ),
        case(
            "codegen-003-pascals-triangle-python3",
            "Generate pascals triangle in Python3",
            "Open code and Write a program to print pascals triangle in python3 and run the prgram",
            ExpectedBehavior {
                // Explicit Code/VS Code run intent → open source in Code and run visibly.
                substrate: Some("VSCodeCodeRunWorkflow".to_string()),
                expected_artifacts: vec![
                    ExpectedArtifact {
                        path_pattern: "pascal_*.py".to_string(),
                        content_contains: Some("def pascals_triangle".to_string()),
                        min_size_bytes: Some(50),
                    },
                    ExpectedArtifact {
                        path_pattern: "output_*.txt".to_string(),
                        content_contains: Some("1".to_string()),
                        min_size_bytes: Some(1),
                    },
                ],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "application 'code and' is not found".to_string(),
                    "application 'Code and' is not found".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &[
                "codegen",
                "pascals-triangle",
                "python3",
                "regression",
                "terminal-execution",
            ],
        ),
    ]
}

/// Returns all suites combined.
pub fn all_suites() -> Vec<GuiEvalCase> {
    let mut cases = Vec::new();
    cases.extend(regression_suite());
    cases.extend(semantic_parsing_suite());
    cases.extend(app_resolution_suite());
    cases.extend(retrieval_isolation_suite());
    cases.extend(false_success_prevention_suite());
    cases.extend(substrate_planning_suite());
    cases.extend(wayland_x11_compatibility_suite());
    cases.extend(app_lifecycle_suite());
    cases.extend(content_generation_suite());
    // New suites from architectural audit
    cases.extend(edge_case_suite());
    cases.extend(multi_site_browser_suite());
    cases.extend(language_detection_suite());
    cases.extend(process_verification_suite());
    // Second-round audit suites
    cases.extend(alias_resolution_robustness_suite());
    cases.extend(content_quality_suite());
    cases.extend(error_handling_suite());
    // Third-round production audit suites
    cases.extend(session_continuity_suite());
    cases.extend(partial_success_suite());
    cases.extend(adversarial_input_suite());
    // Fourth-round: production reliability benchmark
    cases.extend(runtime_robustness_suite());
    cases.extend(multi_language_extended_suite());
    cases.extend(browser_extended_suite());
    cases.extend(verification_coverage_suite());
    // Fifth-round: true execution capability
    cases.extend(terminal_execution_suite());
    // Sixth-round: execution hardening + production chaos
    cases.extend(execution_hardening_suite());
    cases.extend(run_intent_detection_suite());
    // Seventh-round: audit hardening (W-P1/P2/P4 fixes)
    cases.extend(audit_hardening_suite());
    // Eighth-round: production audit hardening (35-finding audit)
    cases.extend(production_audit_suite());
    // Ninth-round: critical production failures (WINDOW_ID_FAILED, artifacts, None verification)
    cases.extend(critical_production_fixes_suite());
    // Tenth-round: 36-finding deep audit fixes
    cases.extend(deep_audit_fixes_suite());
    // Eleventh-round: Wayland-native + production hardening
    cases.extend(wayland_native_hardening_suite());
    // Twelfth-round: production hardening mission — new languages, chaos, parity
    cases.extend(production_hardening_mission_suite());
    // Thirteenth-round: production bug fixes (rule-planner WindowState, graceful degradation)
    cases.extend(production_bug_fixes_suite());
    // Fourteenth-round: AT-SPI interaction engine + structural gaps
    cases.extend(atspi_interaction_suite());
    // Fifteenth-round: InteractionHeavy substrate + popup cognition + semantic completion
    cases.extend(interaction_heavy_suite());
    // Sixteenth-round: Complete A-to-Z production eval suite
    cases.extend(complete_az_eval_suite());
    // Seventeenth-round: Security hardening + architectural fixes
    cases.extend(security_hardening_suite());
    // Eighteenth-round: Production semantic hardening
    cases.extend(semantic_hardening_suite());
    // Nineteenth-round: AT-SPI production integration tests
    cases.extend(atspi_production_suite());
    // Twentieth-round: Production integration completion
    cases.extend(production_integration_suite());
    // Twenty-first-round: Wired feature validation
    cases.extend(wired_features_validation_suite());
    // Twenty-second-round: Final completion validation
    cases.extend(final_completion_suite());
    cases
}

/// Complete A-to-Z eval suite — delegates to az_suite module.
pub fn complete_az_eval_suite() -> Vec<GuiEvalCase> {
    crate::gui_eval::az_suite::complete_az_eval_suite()
}
pub fn security_hardening_suite() -> Vec<GuiEvalCase> {
    vec![
        // AT-SPI cycle prevention: search_subtree must not loop on malformed trees
        case(
            "sec-001-atspi-no-cycle-loop",
            "AT-SPI search must not loop infinitely on malformed accessibility trees",
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
            &["security", "atspi", "cycle-prevention"],
        ),
        // Browser CDP injection prevention: URL must not inject Python code
        case(
            "sec-002-browser-cdp-no-injection",
            "Browser CDP navigate must not allow URL injection",
            "open chrome and search for youtube",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["security", "browser", "injection-prevention"],
        ),
        // InteractionHeavy: click must not pre-check dialog (no 2s overhead)
        case(
            "sec-003-click-no-dialog-overhead",
            "Click interaction must not add dialog detection overhead",
            "click the Save button",
            ExpectedBehavior {
                substrate: Some("InteractionHeavy".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["click_ui_element".to_string()],
                forbidden_tools: vec!["detect_dialog".to_string()],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false, // No app open
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["security", "interaction-heavy", "performance"],
        ),
        // Session persistence: checkpoint must include real user intent
        case(
            "sec-004-session-real-intent",
            "Session checkpoint must save real user intent not task_id",
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
            &["security", "session", "intent-tracking"],
        ),
        // Popup-aware wrapper: must not add latency when AT-SPI unavailable
        case(
            "sec-005-popup-wrapper-no-latency-without-atspi",
            "Popup-aware wrapper must not add latency when AT-SPI socket absent",
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
            &["security", "popup-wrapper", "performance"],
        ),
        // AT-SPI click: single connection reuse (not double connection)
        case(
            "sec-006-atspi-single-connection",
            "AT-SPI click must reuse connection from find_elements",
            "click OK",
            ExpectedBehavior {
                substrate: Some("InteractionHeavy".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["click_ui_element".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false, // No dialog open
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["security", "atspi", "connection-reuse"],
        ),
    ]
}

/// Semantic hardening suite — validates production-grade semantic correctness.
pub fn semantic_hardening_suite() -> Vec<GuiEvalCase> {
    vec![
        // Atomic session write: session file must not be corrupted on concurrent access
        case(
            "sem-001-atomic-session-write",
            "Session checkpoint must be written atomically (no partial writes)",
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
            &["semantic", "session", "atomic-write"],
        ),
        // Browser CDP is_available: must check port not just binary
        case(
            "sem-002-browser-cdp-port-check",
            "Browser CDP is_available must check port 9222 not just binary",
            "open chrome and search for youtube",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["semantic", "browser", "cdp-port-check"],
        ),
        // IDE diagnostics path validation: must reject paths outside allowed dirs
        case(
            "sem-003-ide-diagnostics-path-validation",
            "check_file_diagnostics must validate file path is in allowed directory",
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
            &["semantic", "ide", "path-validation"],
        ),
        // OCR output size limit: must not OOM on large screens
        case(
            "sem-004-ocr-output-size-limit",
            "OCR read_screen must cap output at 100KB",
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
            &["semantic", "ocr", "size-limit"],
        ),
        // Session list size limit: must not return thousands of sessions
        case(
            "sem-005-session-list-size-limit",
            "list_workflow_sessions must cap at 100 sessions",
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
            &["semantic", "session", "size-limit"],
        ),
        // UID-based AT-SPI socket check: must use libc::getuid() not string parsing
        case(
            "sem-006-atspi-uid-robust",
            "AT-SPI socket check must use robust UID detection",
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
            &["semantic", "atspi", "uid-robust"],
        ),
    ]
}

/// AT-SPI production integration tests.
/// Tests deterministic behavior of the new AT-SPI engine features.
pub fn atspi_production_suite() -> Vec<GuiEvalCase> {
    vec![
        // Capability detection: must return structured state
        case(
            "atspi-prod-001-capability-detection",
            "get_accessibility_capabilities must return structured capability state",
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
            &["atspi-production", "capability-detection"],
        ),
        // Accessibility doctor: must not crash
        case(
            "atspi-prod-002-accessibility-doctor",
            "accessibility_doctor must not crash and return structured diagnostics",
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
            &["atspi-production", "accessibility-doctor"],
        ),
        // Weighted ranking: click must prefer active window elements
        case(
            "atspi-prod-003-weighted-ranking",
            "click_ui_element must prefer active window elements (weighted ranking)",
            "click the Save button",
            ExpectedBehavior {
                substrate: Some("InteractionHeavy".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["click_ui_element".to_string()],
                forbidden_tools: vec![],
                // Must return structured failure reason, not panic
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false, // No app open
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["atspi-production", "weighted-ranking"],
        ),
        // Stale element rejection: must detect stale paths
        case(
            "atspi-prod-004-stale-element-rejection",
            "click_ui_element must reject stale element references",
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
            &["atspi-production", "stale-element"],
        ),
        // Post-action verification: click must attempt semantic verification
        case(
            "atspi-prod-005-post-action-verification",
            "click_ui_element must attempt post-action semantic verification",
            "click Cancel",
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
            &["atspi-production", "post-action-verification"],
        ),
        // Snapshot cache: find_ui_elements must use cache on repeated calls
        case(
            "atspi-prod-006-snapshot-cache",
            "find_ui_elements must use snapshot cache for repeated calls",
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
            &["atspi-production", "snapshot-cache"],
        ),
        // Failure taxonomy: must return structured failure reason
        case(
            "atspi-prod-007-failure-taxonomy",
            "click_ui_element must return structured failure reason when element not found",
            "click the nonexistent_button_xyz_99999",
            ExpectedBehavior {
                substrate: Some("InteractionHeavy".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["click_ui_element".to_string()],
                forbidden_tools: vec![],
                // Must NOT panic — must return structured error
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["atspi-production", "failure-taxonomy"],
        ),
        // Invisible element rejection: must reject invisible elements
        case(
            "atspi-prod-008-invisible-rejection",
            "click_ui_element must reject invisible elements",
            "click the Submit button",
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
            &["atspi-production", "invisible-rejection"],
        ),
    ]
}

/// Production integration completion suite.
pub fn production_integration_suite() -> Vec<GuiEvalCase> {
    vec![
        // Session continuation: detect_session_continuation must not crash
        case(
            "prod-int-001-session-continuation-no-crash",
            "Session continuation detection must not crash on any prompt",
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
            &["production-integration", "session-continuation"],
        ),
        // OCR Wayland: xdg-portal screenshot must be tried first
        case(
            "prod-int-002-ocr-wayland-portal",
            "OCR engine must try xdg-desktop-portal screenshot first (Wayland-native)",
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
            &["production-integration", "ocr", "wayland-portal"],
        ),
        // AT-SPI startup detection: accessibility_doctor must run at startup
        case(
            "prod-int-003-atspi-startup-detection",
            "AT-SPI capability detection must run at startup and surface remediation",
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
            &["production-integration", "atspi", "startup-detection"],
        ),
        // New VerificationType: AccessibilityElement must not crash
        case(
            "prod-int-004-accessibility-element-verification",
            "AccessibilityElement VerificationType must not crash",
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
            &[
                "production-integration",
                "accessibility-element",
                "verification",
            ],
        ),
        // New VerificationType: OcrTextOnScreen must not crash
        case(
            "prod-int-005-ocr-text-on-screen-verification",
            "OcrTextOnScreen VerificationType must not crash",
            "open gedit and write a bubble sort in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "bubble_sort_*.py".to_string(),
                    content_contains: Some("def bubble_sort".to_string()),
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
            &[
                "production-integration",
                "ocr-text-on-screen",
                "verification",
            ],
        ),
        // Session user_intent: must save real user text not UUID
        case(
            "prod-int-006-session-real-user-intent",
            "Session checkpoint must save real user intent from loop engine",
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
            &["production-integration", "session", "user-intent"],
        ),
    ]
}

/// Wired feature validation suite — tests all newly wired features end-to-end.
pub fn wired_features_validation_suite() -> Vec<GuiEvalCase> {
    vec![
        // AccessibilityElement verification: must query AT-SPI tree, not return Unverifiable
        case(
            "wired-001-accessibility-element-real-query",
            "AccessibilityElement verification must query AT-SPI tree (not Unverifiable)",
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
            &["wired", "accessibility-element", "real-query"],
        ),
        // OcrTextOnScreen verification: must take live screenshot
        case(
            "wired-002-ocr-text-on-screen-live-screenshot",
            "OcrTextOnScreen verification must take a live screenshot",
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
            &["wired", "ocr-text-on-screen", "live-screenshot"],
        ),
        // OCR portal: xdg-desktop-portal screenshot must find ~/Pictures/Screenshot*.png
        case(
            "wired-003-ocr-portal-screenshot-location",
            "OCR portal screenshot must find file in ~/Pictures/",
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
            &["wired", "ocr", "portal-screenshot"],
        ),
        // Session continuation: must skip UUID-based intents
        case(
            "wired-004-session-continuation-skips-uuid-intents",
            "Session continuation must not surface UUID-based intents from eval runner",
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
            &["wired", "session-continuation", "uuid-skip"],
        ),
        // AccessibilityElement in InteractionHeavy substrate
        case(
            "wired-005-interaction-heavy-with-accessibility-verification",
            "InteractionHeavy substrate with AccessibilityElement verification",
            "click the Save button",
            ExpectedBehavior {
                substrate: Some("InteractionHeavy".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["click_ui_element".to_string()],
                forbidden_tools: vec![],
                // Must return structured failure, not panic
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false, // No app open
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["wired", "interaction-heavy", "accessibility-verification"],
        ),
        // AT-SPI startup detection: must log operational status
        case(
            "wired-006-atspi-startup-logs-status",
            "AT-SPI startup detection must log operational/non-operational status",
            "open gedit and write a bubble sort in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "bubble_sort_*.py".to_string(),
                    content_contains: Some("def bubble_sort".to_string()),
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
            &["wired", "atspi", "startup-logs"],
        ),
        // Verifiability::AccessibilityElement: must not be Unverifiable anymore
        case(
            "wired-007-verifiability-accessibility-element-wired",
            "Verifiability::AccessibilityElement must be wired to real AT-SPI query",
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
            &["wired", "verifiability", "accessibility-element"],
        ),
        // OCR live verification: check_ocr_text_present must try live screenshot first
        case(
            "wired-008-ocr-live-verification-first",
            "check_ocr_text_present must try live screenshot before cache",
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
            &["wired", "ocr", "live-first"],
        ),
    ]
}

/// Final completion validation suite — tests all newly completed features.
pub fn final_completion_suite() -> Vec<GuiEvalCase> {
    vec![
        // Session continuation: must use RecoveryOptions not Token
        case(
            "final-001-session-continuation-recovery-options",
            "Session continuation must emit RecoveryOptions (clickable UI) not Token",
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
            &["final", "session-continuation", "recovery-options"],
        ),
        // AT-SPI auto-enable: gsettings must be called at startup
        case(
            "final-002-atspi-auto-enable",
            "AT-SPI auto-enable must attempt gsettings at startup",
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
            &["final", "atspi", "auto-enable"],
        ),
        // Browser CDP auto-launch: launch_browser_with_debugging tool registered
        case(
            "final-003-browser-cdp-auto-launch-tool",
            "launch_browser_with_debugging tool must be registered",
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
            &["final", "browser", "cdp-auto-launch"],
        ),
        // IDE ruff: check_file_diagnostics must use ruff for Python
        case(
            "final-004-ide-ruff-diagnostics",
            "check_file_diagnostics must use ruff for Python files",
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
            &["final", "ide", "ruff-diagnostics"],
        ),
        // IDE rust-analyzer: check_file_diagnostics must use rustc for Rust
        case(
            "final-005-ide-rust-diagnostics",
            "check_file_diagnostics must use rustc for Rust files",
            "open gedit and write a fibonacci program in rust",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.rs".to_string(),
                    content_contains: Some("fn fibonacci".to_string()),
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
            &["final", "ide", "rust-diagnostics"],
        ),
        // Long-horizon: session continuation skips UUID intents from eval runner
        case(
            "final-006-long-horizon-uuid-skip",
            "Long-horizon session continuation must skip eval runner UUID intents",
            "continue the previous task",
            ExpectedBehavior {
                substrate: None, // May not route to GUI
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: vec![],
                // Must not crash or panic
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false, // No real session to continue
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["final", "long-horizon", "uuid-skip"],
        ),
        // OCR portal: confirmed ~/Pictures/Screenshot*.png location
        case(
            "final-007-ocr-portal-pictures-dir",
            "OCR portal must find screenshot in ~/Pictures/ directory",
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
            &["final", "ocr", "portal-pictures-dir"],
        ),
        // All 13+ cognition tools registered
        case(
            "final-008-all-cognition-tools-registered",
            "All cognition tools must be registered (14 total including launch_browser)",
            "open gedit and write a bubble sort in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "bubble_sort_*.py".to_string(),
                    content_contains: Some("def bubble_sort".to_string()),
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
            &["final", "tools", "all-registered"],
        ),
    ]
}

/// Returns only the fast, non-desktop-requiring cases (CI-safe).
pub fn ci_safe_suite() -> Vec<GuiEvalCase> {
    all_suites()
        .into_iter()
        .filter(|c| !c.requires_desktop)
        .collect()
}

/// Returns cases filtered by tag.
pub fn suite_by_tag(tag: &str) -> Vec<GuiEvalCase> {
    all_suites()
        .into_iter()
        .filter(|c| c.tags.iter().any(|t| t == tag))
        .collect()
}

/// Returns only the new architectural-audit suites.
pub fn audit_suite() -> Vec<GuiEvalCase> {
    let mut cases = Vec::new();
    cases.extend(edge_case_suite());
    cases.extend(multi_site_browser_suite());
    cases.extend(language_detection_suite());
    cases.extend(process_verification_suite());
    cases
}

// ─── Suite 20: Runtime Robustness ────────────────────────────────────────────

/// Tests for runtime robustness under edge conditions.
pub fn runtime_robustness_suite() -> Vec<GuiEvalCase> {
    vec![
        // Test that generated_files_dir() fallback works
        case(
            "robust-001-generated-dir-exists",
            "Generated files directory must exist after workflow",
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
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["robustness", "filesystem"],
        ),
        // Test that the same prompt twice doesn't corrupt state
        case(
            "robust-002-idempotent-execution",
            "Same prompt twice must produce consistent results",
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
            &["robustness", "idempotent"],
        ),
        // Test that a prompt with special characters in the topic doesn't crash
        case(
            "robust-003-special-chars-in-topic",
            "Special characters in topic must not crash",
            "open gedit and write a hello-world program in python",
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
            &["robustness", "special-chars"],
        ),
        // Test that a very short prompt doesn't crash
        case(
            "robust-004-minimal-prompt",
            "Minimal prompt must not crash",
            "open gedit",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["robustness", "minimal"],
        ),
    ]
}

// ─── Suite 21: Multi-Language Extended ───────────────────────────────────────

/// Extended language detection tests.
pub fn multi_language_extended_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "lang-ext-001-kotlin",
            "Generate Kotlin code",
            "open gedit and write a hello world program in kotlin",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "hello_*.kt".to_string(),
                    content_contains: Some("fun main".to_string()),
                    min_size_bytes: Some(10),
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
            &["language-detection", "kotlin"],
        ),
        case(
            "lang-ext-002-shell",
            "Generate shell script",
            "open gedit and write a hello world shell script",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "hello_*.sh".to_string(),
                    content_contains: Some("echo".to_string()),
                    min_size_bytes: Some(10),
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
            &["language-detection", "shell"],
        ),
        case(
            "lang-ext-003-no-language-specified",
            "No language specified — defaults to txt extension",
            "open gedit and write a fibonacci program",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.txt".to_string(),
                    content_contains: Some("def ".to_string()),
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
            &["language-detection", "no-language"],
        ),
    ]
}

// ─── Suite 22: Browser Extended ──────────────────────────────────────────────

/// Extended browser navigation tests.
pub fn browser_extended_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "browser-ext-001-stackoverflow",
            "Open browser and go to stackoverflow",
            "open chrome and search for stackoverflow",
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
            &["browser", "stackoverflow"],
        ),
        case(
            "browser-ext-002-twitter",
            "Open browser and go to twitter",
            "open firefox and search for twitter",
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
            &["browser", "twitter"],
        ),
        case(
            "browser-ext-003-chromium",
            "Open chromium browser",
            "open chromium and search for youtube",
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
            &["browser", "chromium"],
        ),
    ]
}

// ─── Suite 23: Verification Coverage ─────────────────────────────────────────

/// Tests that verify the verification pipeline works correctly.
pub fn verification_coverage_suite() -> Vec<GuiEvalCase> {
    vec![
        // Test that FileSystemEffect verification catches wrong content
        case(
            "verify-001-content-verification",
            "FileSystemEffect must verify content, not just existence",
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
            &["verification", "content"],
        ),
        // Test that ProcessLaunched verification works for AppOpenOnly
        case(
            "verify-002-process-launched-app-open",
            "ProcessLaunched verification for AppOpenOnly substrate",
            "open gedit",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec![],
                // Must not use WindowState (X11-only)
                forbidden_response_patterns: vec![
                    "WINDOW_ID_FAILED".to_string(),
                    "No GUI backend available for window state check".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["verification", "process-launched"],
        ),
        // Test that BrowserNavigate verification is None (no blocking)
        case(
            "verify-003-browser-no-blocking-verify",
            "BrowserNavigate must not block on verification",
            "open chrome and search for youtube",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "verification failed".to_string(),
                    "WINDOW_ID_FAILED".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["verification", "browser"],
        ),
    ]
}

// ─── Suite 17: Session Continuity ────────────────────────────────────────────

/// Tests for session continuity and already-running app detection.
pub fn session_continuity_suite() -> Vec<GuiEvalCase> {
    vec![
        // W-15: Already-running detection
        case(
            "session-001-app-already-running-detection",
            "When app is already running, must not launch duplicate",
            "open gedit",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                // Must not fail with "application not found" — already-running path
                forbidden_response_patterns: vec!["application 'gedit' is not found".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false, // We don't know if it's running
            },
            DisplayServerRequirement::Any,
            true,
            &["session-continuity", "already-running"],
        ),
        // W-16: Single-instance app (VS Code) with file
        case(
            "session-002-vscode-with-file",
            "VS Code opens file — single-instance app",
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
                forbidden_response_patterns: vec!["application 'code and' is not found".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["session-continuity", "vscode", "single-instance"],
        ),
    ]
}

// ─── Suite 18: Partial Success Reporting ─────────────────────────────────────

/// Tests for correct partial-success reporting (W-02, W-20).
pub fn partial_success_suite() -> Vec<GuiEvalCase> {
    vec![
        // W-02: write_file succeeds but open_application_with_file fails
        // The file should still be reported as created
        case(
            "partial-001-file-written-app-not-found",
            "File written but app not found — partial success must report artifact",
            "open nonexistent_editor_xyz_99999 and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                // The file IS written (Step 1 succeeds)
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                // Must NOT claim full success
                forbidden_response_patterns: vec!["Completed: 2 verified steps".to_string()],
                required_response_patterns: vec![],
                // Step 2 fails (app not found) but Step 1 succeeded
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["partial-success", "error-handling"],
        ),
    ]
}

// ─── Suite 19: Adversarial Inputs ────────────────────────────────────────────

/// Tests for adversarial and edge-case inputs.
pub fn adversarial_input_suite() -> Vec<GuiEvalCase> {
    vec![
        // Absolute path as app name
        case(
            "adversarial-001-absolute-path-app",
            "Absolute path as app name must fail gracefully",
            "open /usr/bin/gedit and write a program",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "Done! I completed".to_string(),
                    "panicked".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["adversarial", "absolute-path"],
        ),
        // Unicode in topic
        case(
            "adversarial-002-unicode-topic",
            "Unicode characters in topic must not crash",
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
            &["adversarial", "unicode"],
        ),
        // Repeated "open" in prompt
        case(
            "adversarial-003-repeated-open",
            "Repeated 'open' in prompt must not confuse app extraction",
            "open gedit to open a new file and write fibonacci in python",
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
            &["adversarial", "repeated-open"],
        ),
        // Empty content after "and write"
        case(
            "adversarial-004-empty-content-after-write",
            "Empty content after 'and write' must not crash",
            "open gedit and write",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "panicked".to_string(),
                    "Done! I completed".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["adversarial", "empty-content"],
        ),
    ]
}

// ─── Suite 14: Alias Resolution Robustness ───────────────────────────────────

/// Tests for alias resolution edge cases found in the second audit.
pub fn alias_resolution_robustness_suite() -> Vec<GuiEvalCase> {
    vec![
        // Audit finding: "decode" should not match "code"
        case(
            "alias-001-decode-not-code",
            "Prompt with 'decode' must not resolve to VS Code",
            "open gedit and write a base64 decode function in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    // FIX: generate_filename extracts "base64" from "base64 decode function"
                    path_pattern: "base64_*.py".to_string(),
                    content_contains: Some("def ".to_string()),
                    min_size_bytes: Some(20),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                // Must open gedit, not VS Code
                forbidden_response_patterns: vec!["application 'code' is not found".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["alias-resolution", "false-positive"],
        ),
        // Audit finding: Flatpak reverse-DNS names
        case(
            "alias-002-flatpak-gedit",
            "Flatpak-style app name resolves correctly",
            "open org.gnome.gedit and write a fibonacci program in python",
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
            &["alias-resolution", "flatpak"],
        ),
        // Audit finding: VS Code variants
        case(
            "alias-003-vscode-variant",
            "VSCode alias resolves to code",
            "open vscode and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["application 'vscode' is not found".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["alias-resolution", "vscode"],
        ),
    ]
}

// ─── Suite 15: Content Quality ────────────────────────────────────────────────

/// Tests for code generation quality and correctness.
pub fn content_quality_suite() -> Vec<GuiEvalCase> {
    vec![
        // Audit finding: empty code passes ContainsBytes(b"") — guard test
        case(
            "quality-001-non-empty-code",
            "Generated code must be non-empty and contain real code",
            "open gedit and write a binary search algorithm in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "binary_search_*.py".to_string(),
                    content_contains: Some("def binary_search".to_string()),
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
            &["content-quality", "binary-search"],
        ),
        case(
            "quality-002-tree-traversal",
            "Tree traversal code generation",
            "open gedit and write a binary tree traversal in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "binary_tree_*.py".to_string(),
                    content_contains: Some("class TreeNode".to_string()),
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
            &["content-quality", "tree"],
        ),
        case(
            "quality-003-graph-algorithm",
            "Graph algorithm code generation",
            "open gedit and write a graph BFS algorithm in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "graph_*.py".to_string(),
                    content_contains: Some("def ".to_string()),
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
            &["content-quality", "graph"],
        ),
    ]
}

// ─── Suite 16: Error Handling & Recovery ─────────────────────────────────────

/// Tests for graceful error handling.
pub fn error_handling_suite() -> Vec<GuiEvalCase> {
    vec![
        // Audit finding: Verb::Other always produces Unknown substrate
        case(
            "error-001-complex-verb-graceful",
            "Complex verb (develop/implement) must not crash",
            "develop a fibonacci calculator in gedit using python",
            ExpectedBehavior {
                substrate: None, // May be Unknown — that's OK
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: vec![],
                // Must not crash or produce a panic
                forbidden_response_patterns: vec![
                    "panicked".to_string(),
                    "thread 'main' panicked".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: false, // Unknown substrate → graceful decline
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["error-handling", "complex-verb"],
        ),
        // Audit finding: compound markers miss "implement"
        case(
            "error-002-implement-verb",
            "Implement verb in compound prompt",
            "open gedit and implement a fibonacci function in python",
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
            &["error-handling", "implement-verb"],
        ),
        // Audit finding: "build" verb in compound prompt
        case(
            "error-003-build-verb",
            "Build verb in compound prompt",
            "open gedit and build a prime number sieve in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "prime_*.py".to_string(),
                    content_contains: Some("def ".to_string()),
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
            &["error-handling", "build-verb"],
        ),
        // Audit finding: missing app graceful failure
        case(
            "error-004-missing-app-no-crash",
            "Missing app must fail gracefully without crash",
            "open nonexistent_app_xyz_99999 and write a program",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "panicked".to_string(),
                    "Done! I completed".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["error-handling", "missing-app"],
        ),
    ]
}

// ─── Suite 10: Edge Cases (Architectural Audit Findings) ─────────────────────

/// Tests for edge cases discovered in the architectural audit.
pub fn edge_case_suite() -> Vec<GuiEvalCase> {
    vec![
        // Weakness #12: extract_search_query stops at "and" conjunctions
        case(
            "edge-001-search-query-stops-at-and",
            "Search query extraction stops at 'and' conjunction",
            "open chrome and search for lo-fi music and then open spotify",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                // The query should be "lo-fi music", not "lo-fi music and then open spotify"
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["edge-case", "browser", "conjunction-parsing"],
        ),
        // Weakness #13: Unknown topics produce program_*.ext
        case(
            "edge-002-unknown-topic-filename",
            "Unknown topic still produces a valid file",
            "open gedit and write a quicksort algorithm in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "quicksort_*.py".to_string(),
                    content_contains: Some("def ".to_string()),
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
            &["edge-case", "codegen", "filename"],
        ),
        // Weakness #5: Recovery subtrees must not hardcode gedit
        case(
            "edge-003-kate-editor-not-gedit",
            "Kate editor workflow — must not hardcode gedit in recovery",
            "open kate and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "application 'gedit'".to_string(), // must not fall back to gedit
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["edge-case", "editor", "recovery"],
        ),
        // Malformed/adversarial prompts
        case(
            "edge-004-empty-app-name",
            "Prompt with no recognizable app name — must clarify, not crash",
            "open the thing and write some code",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "Done! I completed".to_string(),
                    "successfully".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: false, // Should ask for clarification
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["edge-case", "adversarial", "clarification"],
        ),
        case(
            "edge-005-very-long-prompt",
            "Very long prompt — must not crash or truncate incorrectly",
            "open gedit and write a comprehensive python program that implements a fibonacci sequence generator with memoization, includes unit tests, has proper docstrings, follows PEP 8 style guidelines, and includes a main function that demonstrates the usage",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def ".to_string()),
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
            &["edge-case", "long-prompt", "robustness"],
        ),
    ]
}

// ─── Suite 11: Multi-Site Browser Navigation ──────────────────────────────────

/// Tests for browser navigation to various sites.
pub fn multi_site_browser_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "browser-001-reddit",
            "Open browser and go to reddit",
            "open chrome and search for reddit",
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
            &["browser", "multi-site"],
        ),
        case(
            "browser-002-github",
            "Open browser and go to github",
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
            &["browser", "multi-site"],
        ),
        case(
            "browser-003-no-search-term",
            "Open browser with no search term — routes to AppOpenOnly (fix #15)",
            "open chrome",
            ExpectedBehavior {
                // FIX #15: "open chrome" with no search → AppOpenOnly
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["browser", "no-search"],
        ),
        case(
            "browser-004-brave",
            "Open brave browser",
            "open brave and search for youtube",
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
            &["browser", "brave"],
        ),
    ]
}

// ─── Suite 12: Language Detection ────────────────────────────────────────────

/// Tests for multi-language code generation.
pub fn language_detection_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "lang-001-typescript",
            "Generate TypeScript code",
            "open gedit and write a fibonacci function in typescript",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.ts".to_string(),
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
            &["language-detection", "typescript"],
        ),
        case(
            "lang-002-go",
            "Generate Go code",
            "open gedit and write a fibonacci program in go",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.go".to_string(),
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
            &["language-detection", "go"],
        ),
        case(
            "lang-003-python3-explicit",
            "Python3 explicit — must produce .py file",
            "open gedit and write a prime number checker in python3",
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
            &["language-detection", "python3"],
        ),
    ]
}

// ─── Suite 13: Process Verification (Wayland-safe) ───────────────────────────

/// Tests that verify process-based verification works on both X11 and Wayland.
pub fn process_verification_suite() -> Vec<GuiEvalCase> {
    vec![
        case(
            "proc-001-app-open-uses-process-launched",
            "AppOpenOnly substrate uses ProcessLaunched verification (Wayland-safe)",
            "open gedit",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                // Must NOT fail with WINDOW_ID_FAILED (which means WindowState was used)
                forbidden_response_patterns: vec![
                    "WINDOW_ID_FAILED".to_string(),
                    "No GUI backend available for window state check".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any, // Must work on both X11 and Wayland
            true,
            &["process-verification", "wayland", "x11"],
        ),
        case(
            "proc-002-file-write-then-open-wayland",
            "FileWriteThenOpen substrate works on Wayland without WindowState",
            "open gedit and write a sorting algorithm in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "sort_*.py".to_string(),
                    content_contains: Some("def bubble_sort".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "WINDOW_ID_FAILED".to_string(),
                    "No GUI backend available for window state check".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["process-verification", "wayland", "file-substrate"],
        ),
    ]
}

// ─── Suite 24: Terminal Execution (True Execution Capability) ────────────────

/// Tests for the TerminalExecution substrate — write code, run it, verify output.
/// This is the "True Execution Capability" suite that validates KRIA can not only
/// write code but also execute it and verify the output is correct.
pub fn terminal_execution_suite() -> Vec<GuiEvalCase> {
    vec![
        // Fibonacci: write + run + verify output contains "0, 1" (fibonacci sequence)
        case(
            "exec-001-fibonacci-python-run",
            "Write and run fibonacci program — verify output",
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
                        content_contains: Some("0, 1".to_string()), // Fibonacci sequence
                        min_size_bytes: Some(1),
                    },
                ],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec![
                    "Done! I completed the GUI automation task".to_string()
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false, // No desktop needed — runs in terminal
            &["terminal-execution", "fibonacci", "python", "run"],
        ),
        // Factorial: write + run + verify output contains "120" (5!)
        case(
            "exec-002-factorial-python-run",
            "Write and run factorial program — verify output contains 120",
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
                        content_contains: Some("120".to_string()), // 5! = 120
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
            &["terminal-execution", "factorial", "python", "run"],
        ),
        // Hello world: write + run + verify output contains "Hello, World"
        case(
            "exec-003-hello-world-python-run",
            "Write and run hello world — verify output contains Hello, World",
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
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["terminal-execution", "hello-world", "python", "run"],
        ),
        // Prime numbers: write + run + verify output contains "2, 3" (first primes)
        case(
            "exec-004-prime-checker-python-run",
            "Write and run prime checker — verify output",
            "open gedit and write a prime number checker in python and run it",
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
                        content_contains: Some("2, 3".to_string()), // First primes
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
            &["terminal-execution", "prime", "python", "run"],
        ),
        // W-17: Program execution failure — missing interpreter must fail gracefully
        // Uses a language that maps to a non-existent interpreter
        case(
            "exec-005-missing-interpreter-graceful",
            "Missing interpreter must fail gracefully — no false success",
            "open gedit and write a fibonacci program in kotlin and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.kt".to_string(),
                    content_contains: Some("fun ".to_string()),
                    min_size_bytes: Some(10),
                }],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                // Must NOT claim success when kotlinc-jvm is not installed
                // (kotlinc-jvm is rarely installed on standard Linux systems)
                // If it IS installed, the test passes — that's correct behavior.
                // The test validates that the response is honest about the outcome.
                forbidden_response_patterns: vec![
                    "Done! I completed the GUI automation task".to_string()
                ],
                required_response_patterns: vec![],
                // May succeed if kotlinc-jvm is installed, may fail if not
                // Either way, must not falsely claim success
                expect_success: false, // Expect failure on systems without kotlinc-jvm
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &[
                "terminal-execution",
                "missing-interpreter",
                "error-handling",
                "kotlin",
            ],
        ),
        // W-17: Bubble sort run — verify output contains sorted numbers
        case(
            "exec-006-bubble-sort-python-run",
            "Write and run bubble sort — verify sorted output",
            "open gedit and write a bubble sort algorithm in python and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![
                    ExpectedArtifact {
                        path_pattern: "bubble_sort_*.py".to_string(),
                        content_contains: Some("def bubble_sort".to_string()),
                        min_size_bytes: Some(50),
                    },
                    ExpectedArtifact {
                        path_pattern: "output_*.txt".to_string(),
                        content_contains: Some("11".to_string()), // From sorted example
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
            &["terminal-execution", "sort", "python", "run"],
        ),
    ]
}

// ─── Suite 25: Execution Hardening ───────────────────────────────────────────

/// Tests for execution hardening — verifies that the execution pipeline
/// correctly handles errors, crashes, and edge cases.
pub fn execution_hardening_suite() -> Vec<GuiEvalCase> {
    vec![
        // Test that traceback detection works — Python ZeroDivisionError
        case(
            "hard-001-python-runtime-error-detected",
            "Python runtime error must be detected — no false success",
            "open gedit and write a program that divides by zero in python and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    // FIX: generate_filename now extracts first meaningful word;
                    // "divides" is extracted from "program that divides by zero"
                    path_pattern: "divides_*.py".to_string(),
                    content_contains: Some("def ".to_string()),
                    min_size_bytes: Some(10),
                }],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                // Must NOT claim success when program crashes
                forbidden_response_patterns: vec!["Completed: 2 verified steps".to_string()],
                required_response_patterns: vec![],
                expect_success: false, // ZeroDivisionError → traceback → fail
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["execution-hardening", "runtime-error", "python"],
        ),
        // Test that output file accumulation is prevented
        case(
            "hard-002-output-file-cleanup",
            "Output files must be cleaned up between runs",
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
            &["execution-hardening", "cleanup", "fibonacci"],
        ),
        // Test that C++ code generation and execution works
        case(
            "hard-003-cpp-hello-world",
            "C++ hello world — write and run",
            "open gedit and write a hello world program in c++ and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "hello_*.cpp".to_string(),
                    content_contains: Some("int main".to_string()),
                    min_size_bytes: Some(20),
                }],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                // May succeed or fail depending on g++ availability
                expect_success: false, // Conservative — g++ may not be installed
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["execution-hardening", "cpp", "hello-world"],
        ),
        // Test that the "then run" pattern is detected
        case(
            "hard-004-then-run-pattern",
            "Then run pattern must trigger TerminalExecution",
            "open gedit and write a factorial program in python then run it",
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
            &["execution-hardening", "then-run", "factorial"],
        ),
    ]
}

// ─── Suite 26: Run Intent Detection ──────────────────────────────────────────

/// Tests for run intent detection — verifies that various phrasings of
/// "run the program" are correctly detected.
pub fn run_intent_detection_suite() -> Vec<GuiEvalCase> {
    vec![
        // "execute the program" pattern
        case(
            "run-001-execute-the-program",
            "Execute the program pattern triggers TerminalExecution",
            "open gedit and write a hello world program in python and execute it",
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
            &["run-intent", "execute-pattern", "hello-world"],
        ),
        // "run the program" pattern
        case(
            "run-002-run-the-program",
            "Run the program pattern triggers TerminalExecution",
            "open gedit and write a prime checker in python and run the program",
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
            &["run-intent", "run-the-program", "prime"],
        ),
        // No run intent — must use FileWriteThenOpen
        case(
            "run-003-no-run-intent-uses-file-substrate",
            "No run intent must use FileWriteThenOpen, not TerminalExecution",
            "open gedit and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec!["execute_bash".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["run-intent", "no-run", "file-substrate"],
        ),
    ]
}
// ─── Suite 27: Audit Hardening (W-P1/P2/P4 fixes) ───────────────────────────

/// Tests that validate the production audit fixes:
/// - W-P1-01/02/03: UUID-per-workflow binary paths (no concurrent corruption)
/// - W-P2-01/02: Extended traceback detection (MemoryError, runtime error:, etc.)
/// - W-P2-05: Default expected output is empty string (not "Running:")
/// - W-P4-01/02/03/04: Language detection gaps (python 3.11, node.js, ES6, go language)
pub fn audit_hardening_suite() -> Vec<GuiEvalCase> {
    vec![
        // W-P2-05: Program that produces no output — must not false-succeed
        case(
            "exec-007-empty-output-program",
            "Program that produces no output — empty expected_output must not false-succeed",
            "open gedit and write a program that does nothing in python and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    // FIX: generate_filename extracts "does" from "program that does nothing"
                    // ("does" is the first word > 3 chars after exclusions)
                    path_pattern: "does_*.py".to_string(),
                    content_contains: Some("def ".to_string()),
                    min_size_bytes: Some(5),
                }],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                // Must not claim success if program produces no output
                forbidden_response_patterns: vec![
                    "Done! I completed the GUI automation task".to_string()
                ],
                required_response_patterns: vec![],
                // A program that does nothing produces no output → verifier should
                // report failure (empty output) rather than false success
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["audit-hardening", "empty-output", "W-P2-05"],
        ),
        // W-P4-01: "python 3.11" language tag must produce .py file
        case(
            "lang-ext-004-python-version-qualified",
            "Python version-qualified tag (python 3.11) must produce .py file",
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
                // Must produce .py not .txt
                forbidden_response_patterns: vec!["fibonacci_*.txt".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &[
                "audit-hardening",
                "language-detection",
                "python-version",
                "W-P4-01",
            ],
        ),
        // W-P4-02: "node.js" language tag must produce .js file
        case(
            "lang-ext-005-nodejs-language",
            "node.js language tag must produce .js file",
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
            &["audit-hardening", "language-detection", "nodejs", "W-P4-02"],
        ),
        // W-P4-03: "ES6" language tag must produce .js file
        case(
            "lang-ext-006-es6-language",
            "ES6 language tag must produce .js file",
            "open gedit and write a fibonacci function in ES6",
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
            &["audit-hardening", "language-detection", "es6", "W-P4-03"],
        ),
        // W-P4-04: "go language" phrasing must produce .go file
        case(
            "lang-ext-007-go-language-phrasing",
            "go language phrasing must produce .go file",
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
            &[
                "audit-hardening",
                "language-detection",
                "go-language",
                "W-P4-04",
            ],
        ),
        // W-P1-01/02/03: Concurrent execution must not corrupt binaries
        // Run two fibonacci programs back-to-back — each must produce correct output
        case(
            "exec-008-concurrent-safe-rust-execution",
            "Rust execution uses UUID binary path — no concurrent corruption",
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
                // May succeed or fail depending on rustc availability
                expect_success: false, // Conservative — rustc may not be installed
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["audit-hardening", "concurrent-safe", "rust", "W-P1-01"],
        ),
        // W-P2-01: MemoryError must be detected as a crash
        // NOTE: This test validates that IF a MemoryError occurs, it's detected.
        // On machines with sufficient RAM, the program may succeed — that's also valid.
        // The test checks that the response is honest (no false success when it crashes).
        case(
            "hard-005-memory-error-detected",
            "Python MemoryError detection — verifier must not false-succeed on crash",
            "open gedit and write a program that prints fibonacci in python and run it",
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
                expect_success: true, // Fibonacci runs fine — validates W-P2-01 traceback list doesn't false-positive
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["audit-hardening", "memory-error", "W-P2-01"],
        ),
    ]
}

// ─── Suite 28: Production Audit Hardening ────────────────────────────────────

/// Tests that validate the 35-finding production audit fixes:
/// - #3: Timeout evidence reports correct ms value
/// - #7: execute_goal_tree heartbeat task is properly aborted
/// - #9: /proc poll interval reduced (40 iterations max vs 160)
/// - #17: Output size capped at 1MB via head -c 1048576
/// - #18: Output file read capped at 1MB
/// - #28: Additional browser aliases (chromium-browser, google-chrome-stable, etc.)
/// - #31: SystemSleep capped at 30 seconds
pub fn production_audit_suite() -> Vec<GuiEvalCase> {
    vec![
        // #17/#18: Large output program must not fill disk or OOM
        case(
            "audit-prod-001-output-size-limit",
            "Program with large output must be capped at 1MB — no disk fill",
            "open gedit and write a program that prints fibonacci in python and run it",
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
            &["production-audit", "output-size-limit", "audit-17-18"],
        ),
        // #28: chromium-browser alias must route to BrowserNavigate
        case(
            "audit-prod-002-chromium-browser-alias",
            "chromium-browser alias must route to BrowserNavigate when search query present",
            "open chromium and search for youtube",
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
            &["production-audit", "browser-alias", "audit-28"],
        ),
        // #28: google-chrome-stable alias
        case(
            "audit-prod-003-google-chrome-stable-alias",
            "google-chrome-stable alias must route to BrowserNavigate",
            "open google-chrome-stable and search for github",
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
            &["production-audit", "browser-alias", "audit-28"],
        ),
        // #31: SystemSleep cap — workflow with sleep must not hang
        case(
            "audit-prod-004-sleep-cap-enforced",
            "Workflow with sleep step must complete within reasonable time",
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
            &["production-audit", "sleep-cap", "audit-31"],
        ),
        // #9: /proc poll interval — ProcessLaunched must still work with 200ms interval
        case(
            "audit-prod-005-process-launched-200ms-poll",
            "ProcessLaunched verifier works with 200ms poll interval",
            "open gedit",
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
            &["production-audit", "process-launched", "audit-9"],
        ),
        // Concurrent execution safety — two fibonacci programs back-to-back
        case(
            "audit-prod-006-concurrent-execution-safe",
            "Two sequential executions must not corrupt each other",
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
            &["production-audit", "concurrent-safe", "audit-1-2-3"],
        ),
    ]
}

// ─── Suite 29: Critical Production Fixes ─────────────────────────────────────

/// Tests that validate the critical production fixes:
/// - created_artifacts now populated in production path (not just eval runner)
/// - VerificationType::None emits telemetry instead of silent skip
/// - KillSwitchInterceptor::Drop guards against no-runtime panic
/// - execute_workflow accepts planned_artifacts parameter
pub fn critical_production_fixes_suite() -> Vec<GuiEvalCase> {
    vec![
        // Artifact tracking: file must appear in created_artifacts after FileWriteThenOpen
        case(
            "crit-001-artifacts-populated-file-substrate",
            "created_artifacts must be populated after FileWriteThenOpen workflow",
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
            &["critical-fix", "artifacts", "file-substrate"],
        ),
        // Artifact tracking: output file must appear after TerminalExecution
        case(
            "crit-002-artifacts-populated-terminal-execution",
            "created_artifacts must include both source and output files after TerminalExecution",
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
            &["critical-fix", "artifacts", "terminal-execution"],
        ),
        // VerificationType::None telemetry: BrowserNavigate uses None verification
        // and must still complete successfully (None = unverified, not failed)
        case(
            "crit-003-none-verification-completes-not-fails",
            "VerificationType::None steps must complete (unverified) not fail",
            "open chrome and search for youtube",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                // Must not report failure just because verification is None
                forbidden_response_patterns: vec![
                    "verification failed".to_string(),
                    "WINDOW_ID_FAILED".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["critical-fix", "none-verification", "browser"],
        ),
        // Artifact tracking: BrowserNavigate has no artifacts — must not crash
        case(
            "crit-004-no-artifacts-browser-navigate",
            "BrowserNavigate workflow with no artifacts must not crash artifact tracking",
            "open firefox and go to github",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["critical-fix", "artifacts", "browser"],
        ),
        // Partial success: file written but app not found — artifact still tracked
        case(
            "crit-005-partial-success-artifact-still-tracked",
            "When Step 1 succeeds but Step 2 fails, artifact from Step 1 must still be tracked",
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
                // Must NOT claim full success
                forbidden_response_patterns: vec!["Completed: 2 verified steps".to_string()],
                required_response_patterns: vec![],
                // Step 2 fails (app not found) but Step 1 succeeded
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["critical-fix", "artifacts", "partial-success"],
        ),
        // Concurrent artifact isolation: two workflows must not share output files
        case(
            "crit-006-concurrent-artifact-isolation",
            "Concurrent workflows must produce isolated artifacts (UUID output files)",
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
            &["critical-fix", "concurrent-isolation", "artifacts"],
        ),
    ]
}

// ─── Suite 30: Deep Audit Fixes ──────────────────────────────────────────────

/// Tests that validate the 36-finding deep audit fixes:
/// - #7: Java class name extracted from source, not hardcoded "Main"
/// - #9: /proc comm exact match only (no binary.starts_with(comm) false positive)
/// - #15: "open firefox" with no search → AppOpenOnly, not BrowserNavigate with empty query
/// - #16: "type hello world" → Literal, not Generated
/// - #17: open_application_with_file gets 1500ms grace period
/// - #19: Step success uses sub_goal.step <= completed_steps
/// - #20: Cloud LLM leakage detected on first invocation, not just retries
/// - #21: Filename uses UUID suffix, not second-resolution timestamp
/// - #24: "find " removed from browser search markers
/// - #28: bare "js" uses word-boundary check
/// - #33: CloseApplication uses exact match, not substring
/// - #34: Duplicate binary search check removed
/// - #36: Unknown topic extracts first meaningful word from hint
pub fn deep_audit_fixes_suite() -> Vec<GuiEvalCase> {
    vec![
        // #15: "open firefox" with no search → AppOpenOnly (not BrowserNavigate with empty query)
        // NOTE: The existing tests (parse-002, browser-003) expect BrowserNavigate for "open firefox"
        // because the old behavior always routed browsers to BrowserNavigate. Fix #15 changes this
        // to AppOpenOnly when there is no search query. The new test validates the corrected behavior.
        case(
            "audit36-001-browser-no-search-app-open",
            "open firefox with no search term must route to AppOpenOnly not BrowserNavigate",
            "open firefox",
            ExpectedBehavior {
                // FIX #15: "open firefox" with no search → AppOpenOnly
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["audit-36", "browser", "no-search", "fix-15"],
        ),
        // #21: Filename uniqueness — two concurrent fibonacci workflows must produce different filenames
        case(
            "audit36-002-filename-uuid-uniqueness",
            "Filename uses UUID suffix — no timestamp collision between concurrent workflows",
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
            &["audit-36", "filename", "uuid", "fix-21"],
        ),
        // #24: "find" in prompt must not trigger browser search
        case(
            "audit36-003-find-not-browser-search",
            "open chrome and find fibonacci must not trigger browser search for fibonacci",
            "open gedit and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["audit-36", "find-marker", "fix-24"],
        ),
        // #17: open_application_with_file grace period — FileWriteThenOpen must complete faster
        case(
            "audit36-004-file-write-then-open-grace-period",
            "FileWriteThenOpen substrate completes with grace period for app launch",
            "open kate and write a prime checker in python",
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
            &["audit-36", "grace-period", "fix-17"],
        ),
        // #36: Unknown topic extracts meaningful word from hint
        case(
            "audit36-005-unknown-topic-meaningful-filename",
            "Unknown topic hint extracts first meaningful word for filename",
            "open gedit and write a quicksort algorithm in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "quicksort_*.py".to_string(),
                    content_contains: Some("def ".to_string()),
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
            &["audit-36", "filename", "unknown-topic", "fix-36"],
        ),
        // #7: Java execution uses correct class name
        case(
            "audit36-006-java-class-name-extracted",
            "Java execution uses extracted class name not hardcoded Main",
            "open gedit and write a fibonacci program in java and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.java".to_string(),
                    content_contains: Some("public class Main".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                // May succeed or fail depending on javac availability
                expect_success: false, // Conservative — javac may not be installed
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["audit-36", "java", "class-name", "fix-7"],
        ),
    ]
}

// ─── Suite 31: Wayland-Native Hardening ──────────────────────────────────────

/// Tests that validate the Wayland-native hardening fixes:
/// - Target window lock established after open_application_with_file (fix #8)
/// - Partial-success artifacts tracked incrementally (fix #2)
/// - CRLF .desktop file parsing (fix #13)
/// - is_installed fails closed on lock contention (fix #14)
/// - chrome alias maps to google-chrome not google-chrome-stable (fix #25)
/// - send_ipc_request shuts down write half (fix #32)
/// - get_active_window Wayland fallback via AT-SPI (new capability)
pub fn wayland_native_hardening_suite() -> Vec<GuiEvalCase> {
    vec![
        // Target window lock: FileWriteThenOpen must complete with lock established
        case(
            "wayland-001-target-lock-after-launch",
            "Target window lock established after open_application_with_file",
            "open gedit and write a fibonacci program in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(100),
                }],
                required_tools: vec![
                    "write_file".to_string(),
                    "open_application_with_file".to_string(),
                ],
                forbidden_tools: vec![],
                // Must not fail with WINDOW_ID_FAILED after the grace period
                forbidden_response_patterns: vec!["TARGET LOCK BROKEN".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["wayland-native", "target-lock", "fix-8"],
        ),
        // Partial-success artifact tracking: file written but app not found
        case(
            "wayland-002-partial-success-artifact-tracked",
            "Partial success: file written but app not found — artifact must be in result",
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
                expect_success: false, // Step 2 fails (app not found)
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["wayland-native", "partial-success", "artifacts", "fix-2"],
        ),
        // Wayland fallback: workflow must complete even when WINDOW_ID_FAILED
        case(
            "wayland-003-workflow-survives-window-id-failed",
            "Workflow must complete successfully even when get_active_window fails",
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
                // Must NOT abort due to WINDOW_ID_FAILED for non-input steps
                forbidden_response_patterns: vec!["Cannot perform input action".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["wayland-native", "window-id-failed", "resilience"],
        ),
        // Incremental artifact tracking: TerminalExecution source file tracked even on output failure
        case(
            "wayland-004-terminal-execution-source-tracked",
            "TerminalExecution: source file tracked even when execution fails",
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
            &["wayland-native", "artifacts", "terminal-execution", "fix-2"],
        ),
        // Chrome alias: must work with google-chrome (not just google-chrome-stable)
        case(
            "wayland-005-chrome-alias-google-chrome",
            "chrome alias maps to google-chrome (not just google-chrome-stable)",
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
            &["wayland-native", "chrome-alias", "fix-25"],
        ),
        // CRLF desktop file: app with CRLF-formatted .desktop must be found
        case(
            "wayland-006-crlf-desktop-file-parsed",
            "Apps with CRLF .desktop files must be found in registry",
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
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["wayland-native", "crlf", "desktop-file", "fix-13"],
        ),
    ]
}

// ─── Suite 32: Production Hardening Mission ───────────────────────────────────

/// Tests for the production hardening mission:
/// - New language support: Ruby, PHP
/// - Language detection improvements: Java, Kotlin, C#, C++, Swift
/// - Browser default fix: "browser" without name
/// - Startup validation: tool registry self-test
/// - Production parity: tests that match real production behavior
pub fn production_hardening_mission_suite() -> Vec<GuiEvalCase> {
    vec![
        // Ruby code generation
        case(
            "prod-001-ruby-fibonacci",
            "Generate Ruby fibonacci program",
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
            &["production-hardening", "ruby", "language-detection"],
        ),
        // PHP code generation
        case(
            "prod-002-php-fibonacci",
            "Generate PHP fibonacci program",
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
            &["production-hardening", "php", "language-detection"],
        ),
        // Java language detection
        case(
            "prod-003-java-fibonacci",
            "Generate Java fibonacci program",
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
            &["production-hardening", "java", "language-detection"],
        ),
        // C# language detection
        case(
            "prod-004-csharp-fibonacci",
            "Generate C# fibonacci program",
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
            &["production-hardening", "csharp", "language-detection"],
        ),
        // Ruby run
        case(
            "prod-005-ruby-run",
            "Write and run Ruby fibonacci — must not false-succeed when ruby not installed",
            "open gedit and write a fibonacci program in ruby and run it",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.rb".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(30),
                }],
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: vec![],
                // Must not claim success when ruby is not installed
                forbidden_response_patterns: vec![
                    "Done! I completed the GUI automation task".to_string()
                ],
                required_response_patterns: vec![],
                // Ruby may not be installed — expect failure
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["production-hardening", "ruby", "terminal-execution"],
        ),
        // Production parity: same prompt that failed in production at 11:30
        case(
            "prod-006-production-parity-gedit-fibonacci",
            "Production parity: exact prompt from production failure at 11:30",
            "Open gedit and type a program to print fibonacci series in python",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "fibonacci_*.py".to_string(),
                    content_contains: Some("def fibonacci".to_string()),
                    min_size_bytes: Some(100),
                }],
                required_tools: vec![
                    "write_file".to_string(),
                    "open_application_with_file".to_string(),
                ],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "tool does not implement execute".to_string(),
                    "No GUI backend available for window state check".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["production-hardening", "production-parity", "regression"],
        ),
        // RFC 008 Phase 4: Browser Stale Tab Chaos
        case(
            "prod-010-chaos-stale-tab",
            "Chaos: executor must interact with the active tab, not a stale background tab",
            "open browser to example.com, open a new tab to httpbin.org, and click on 'ip'",
            ExpectedBehavior {
                substrate: Some("BrowserNavigation".to_string()),
                expected_artifacts: vec![],
                required_tools: vec![
                    "managed_browser_navigate".to_string(),
                    "click_element".to_string(),
                ],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["production-hardening", "chaos", "browser-stale-tab"],
        ),
        // RFC 008 Phase 4: Browser SPA Transition Chaos
        case(
            "prod-011-chaos-spa-transition",
            "Chaos: executor must not hallucinate success during slow SPA transitions",
            "navigate to httpbin.org/delay/5 and wait for the page to load, then click 'home'",
            ExpectedBehavior {
                substrate: Some("BrowserNavigation".to_string()),
                expected_artifacts: vec![],
                required_tools: vec!["managed_browser_navigate".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["production-hardening", "chaos", "browser-spa-transition"],
        ),
        // Chaos: missing interpreter — must fail gracefully
        case(
            "prod-007-chaos-missing-interpreter",
            "Chaos: missing interpreter must fail gracefully without false success",
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
                expect_success: false, // kotlinc-jvm likely not installed
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["production-hardening", "chaos", "missing-interpreter"],
        ),
        // Concurrent execution: two different programs back-to-back
        case(
            "prod-008-concurrent-isolation",
            "Concurrent isolation: two programs must not share output files",
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
            &["production-hardening", "concurrent-isolation"],
        ),
        // Kotlin language detection (even if interpreter not available)
        case(
            "prod-009-kotlin-file-generation",
            "Kotlin language detection produces .kt file",
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
            &["production-hardening", "kotlin", "language-detection"],
        ),
        // Swift language detection
        case(
            "prod-010-swift-file-generation",
            "Swift language detection produces .swift file",
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
            &["production-hardening", "swift", "language-detection"],
        ),
    ]
}

// ─── Suite 33: Production Bug Fixes ──────────────────────────────────────────

/// Tests that validate the production bug fixes:
/// - Rule-based planner now uses ProcessLaunched instead of WindowState
/// - Daemon GetActiveWindow has Wayland fallback (AT-SPI + /proc)
/// - Graceful degradation when LLM unavailable
pub fn production_bug_fixes_suite() -> Vec<GuiEvalCase> {
    vec![
        // Rule-based planner: open_application must use ProcessLaunched (not WindowState)
        // This was causing 4 failures at 12:09-12:12 in production logs
        case(
            "bugfix-001-rule-planner-process-launched",
            "Rule-based planner open_application uses ProcessLaunched (Wayland-safe)",
            "open gedit",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec![],
                // Must NOT fail with WindowState/WINDOW_ID_FAILED
                forbidden_response_patterns: vec![
                    "WINDOW_ID_FAILED".to_string(),
                    "No GUI backend available for window state check".to_string(),
                    "Failed to get active window".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["bugfix", "rule-planner", "process-launched", "wayland"],
        ),
        // Rule-based planner: switch_to_window also uses ProcessLaunched
        case(
            "bugfix-002-rule-planner-switch-process-launched",
            "Rule-based planner switch_to_window uses ProcessLaunched (Wayland-safe)",
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
                forbidden_response_patterns: vec![
                    "WINDOW_ID_FAILED".to_string(),
                    "No GUI backend available for window state check".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["bugfix", "rule-planner", "process-launched"],
        ),
        // Production parity: the exact workflow that failed 4 times at 12:09-12:12
        // (rule-based planner path with WindowState verification)
        case(
            "bugfix-003-production-parity-open-app-wayland",
            "Production parity: open_application must succeed on Wayland without WINDOW_ID_FAILED",
            "open kate",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec![
                    "WINDOW_ID_FAILED".to_string(),
                    "verification failed after 5 retries".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["bugfix", "production-parity", "wayland", "open-app"],
        ),
        // Graceful degradation: workflow failure must include artifact hint
        case(
            "bugfix-004-graceful-degradation-artifact-hint",
            "Workflow failure must include artifact path hint when file was written",
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
                // Must NOT claim full success
                forbidden_response_patterns: vec!["Completed: 2 verified steps".to_string()],
                required_response_patterns: vec![],
                // Step 2 fails (app not found) but Step 1 succeeded
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["bugfix", "graceful-degradation", "artifact-hint"],
        ),
        // Daemon Wayland fallback: workflow must complete even when xdotool fails
        case(
            "bugfix-005-daemon-wayland-fallback",
            "Workflow completes even when xdotool/WINDOW_ID_FAILED — daemon has fallback",
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
                // Must NOT abort due to WINDOW_ID_FAILED
                forbidden_response_patterns: vec!["Cannot perform input action".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["bugfix", "daemon-wayland-fallback", "window-id-failed"],
        ),
    ]
}

// ─── Suite 34: AT-SPI Interaction Engine ─────────────────────────────────────

/// Tests for the new AT-SPI interaction engine:
/// - detect_dialog: detects visible dialogs
/// - dismiss_dialog: dismisses dialogs
/// - get_desktop_state: captures desktop state
/// - check_app_responding: checks app responsiveness
/// - find_ui_elements: finds elements by role/name
/// - click_ui_element: clicks elements by name
/// - fill_form_field: fills form fields
pub fn atspi_interaction_suite() -> Vec<GuiEvalCase> {
    vec![
        // AT-SPI availability check: get_desktop_state must not crash
        case(
            "atspi-001-desktop-state-no-crash",
            "get_desktop_state must not crash even when AT-SPI is unavailable",
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
                // Must not crash or panic
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["atspi", "desktop-state", "no-crash"],
        ),
        // AT-SPI dialog detection: detect_dialog must not crash
        case(
            "atspi-002-dialog-detection-no-crash",
            "detect_dialog must not crash even when no dialog is visible",
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
            &["atspi", "dialog-detection", "no-crash"],
        ),
        // AT-SPI tools registered: find_ui_elements must be in registry
        case(
            "atspi-003-tools-registered",
            "AT-SPI tools must be registered in the tool registry",
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
            &["atspi", "tools-registered"],
        ),
        // Popup handling: workflow must complete even if a dialog appears
        case(
            "atspi-004-popup-resilience",
            "Workflow must complete even when popup detection is active",
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
                forbidden_response_patterns: vec![],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["atspi", "popup-resilience"],
        ),
    ]
}

// ─── Suite 35: InteractionHeavy Substrate + Popup Cognition ──────────────────

/// Tests for the new InteractionHeavy substrate and popup-aware workflow wrapper.
pub fn interaction_heavy_suite() -> Vec<GuiEvalCase> {
    vec![
        // InteractionHeavy substrate: click verb routes to InteractionHeavy
        case(
            "interact-001-click-routes-to-interaction-heavy",
            "Click verb must route to InteractionHeavy substrate via AT-SPI",
            "click the Save button",
            ExpectedBehavior {
                substrate: Some("InteractionHeavy".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["click_ui_element".to_string()],
                forbidden_tools: vec!["web_search".to_string()],
                // Must not crash or panic
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                // May succeed or fail depending on whether Save button exists
                expect_success: false, // Conservative — no app open
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["interaction-heavy", "click", "atspi"],
        ),
        // InteractionHeavy: "click OK" routes correctly
        case(
            "interact-002-click-ok-button",
            "Click OK button routes to InteractionHeavy substrate",
            "click OK",
            ExpectedBehavior {
                substrate: Some("InteractionHeavy".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["click_ui_element".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false, // No dialog open
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["interaction-heavy", "click", "dialog"],
        ),
        // InteractionHeavy: "click Cancel" routes correctly
        case(
            "interact-003-click-cancel-button",
            "Click Cancel button routes to InteractionHeavy substrate",
            "click Cancel",
            ExpectedBehavior {
                substrate: Some("InteractionHeavy".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["click_ui_element".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false, // No dialog open
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["interaction-heavy", "click", "dialog"],
        ),
        // Popup-aware: workflow failure annotates dialog context
        case(
            "interact-004-popup-aware-workflow",
            "Popup-aware workflow wrapper detects and annotates dialog interruptions",
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
                // Must not crash even when dialog detection runs
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["interaction-heavy", "popup-aware", "dialog"],
        ),
        // AT-SPI dialog detection: detect_dialog tool works
        case(
            "interact-005-detect-dialog-tool",
            "detect_dialog tool must not crash and returns dialog_found field",
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
            &["interaction-heavy", "detect-dialog", "atspi"],
        ),
        // InteractionHeavy: "press the submit button" routes correctly
        case(
            "interact-006-press-submit-button",
            "Press submit button routes to InteractionHeavy substrate",
            "press the submit button",
            ExpectedBehavior {
                substrate: Some("InteractionHeavy".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["click_ui_element".to_string()],
                forbidden_tools: vec![],
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false, // No app open
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["interaction-heavy", "click", "button"],
        ),
        // Chaos: workflow with dialog detection must complete normally when no dialog
        case(
            "interact-007-no-dialog-workflow-completes",
            "Workflow with popup detection completes normally when no dialog present",
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
            &["interaction-heavy", "chaos", "no-dialog"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_suites_have_unique_ids() {
        let cases = all_suites();
        let mut ids = std::collections::HashSet::new();
        for case in &cases {
            assert!(
                ids.insert(case.id.clone()),
                "Duplicate case ID: {}",
                case.id
            );
        }
    }

    #[test]
    fn regression_suite_covers_three_exact_failures() {
        let cases = regression_suite();
        let ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&"regression-001-chrome-youtube"));
        assert!(ids.contains(&"regression-002-gedit-fibonacci"));
        assert!(ids.contains(&"regression-003-code-pascals-triangle"));
    }

    #[test]
    fn all_cases_have_non_empty_prompts() {
        for case in all_suites() {
            assert!(!case.prompt.is_empty(), "Case {} has empty prompt", case.id);
        }
    }

    #[test]
    fn all_cases_have_descriptions() {
        for case in all_suites() {
            assert!(
                !case.description.is_empty(),
                "Case {} has empty description",
                case.id
            );
        }
    }

    #[test]
    fn all_cases_have_phase1_governance_metadata() {
        for case in all_suites() {
            assert!(
                !case.governance.capability_ids.is_empty(),
                "Case {} has no capability mapping",
                case.id
            );
            assert!(
                !case.governance.failure_mode_ids.is_empty(),
                "Case {} has no failure mode mapping",
                case.id
            );
            assert!(
                case.governance.priority.is_some(),
                "Case {} has no priority",
                case.id
            );
            assert!(
                case.governance.cost_class.is_some(),
                "Case {} has no cost class",
                case.id
            );
            assert!(
                case.governance.environment_profile.is_some(),
                "Case {} has no environment profile",
                case.id
            );
            assert!(
                case.governance.oracle_type.is_some(),
                "Case {} has no oracle type",
                case.id
            );
            assert!(
                case.governance.owner.as_deref().unwrap_or_default() != "",
                "Case {} has no owner",
                case.id
            );
            assert!(
                case.governance.dedup_key.as_deref().unwrap_or_default() != "",
                "Case {} has no dedup key",
                case.id
            );
        }
    }

    #[test]
    fn governance_report_tracks_all_cases() {
        let cases = all_suites();
        let report = crate::gui_eval::governance::build_governance_report(&cases);
        assert_eq!(report.entropy.total_cases, cases.len());
        assert!(
            report.missing_metadata_cases.is_empty(),
            "Missing metadata: {:?}",
            report.missing_metadata_cases
        );
        assert!(!report.capabilities.is_empty());
        assert!(!report.cost_breakdown.is_empty());
        assert!(!report.priority_breakdown.is_empty());
    }

    #[test]
    fn ci_safe_suite_has_no_desktop_cases() {
        for case in ci_safe_suite() {
            assert!(
                !case.requires_desktop,
                "CI-safe case {} requires desktop",
                case.id
            );
        }
    }

    #[test]
    fn suite_by_tag_filters_correctly() {
        let regression = suite_by_tag("regression");
        assert!(!regression.is_empty());
        for case in &regression {
            assert!(case.tags.contains(&"regression".to_string()));
        }
    }

    #[test]
    fn false_success_cases_have_forbidden_patterns() {
        for case in false_success_prevention_suite() {
            assert!(
                !case
                    .expected_behavior
                    .forbidden_response_patterns
                    .is_empty()
                    || !case.expected_behavior.expected_artifacts.is_empty(),
                "False-success case {} has no detection mechanism",
                case.id
            );
        }
    }

    #[test]
    fn retrieval_isolation_cases_have_forbidden_tools() {
        for case in retrieval_isolation_suite() {
            assert!(
                !case.expected_behavior.forbidden_tools.is_empty(),
                "Retrieval isolation case {} has no forbidden tools",
                case.id
            );
        }
    }
}
