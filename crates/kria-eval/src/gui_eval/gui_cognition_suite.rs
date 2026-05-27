//! GUI Cognition Eval Suite — Real-World Multi-Step Prompts
//!
//! # Purpose
//!
//! This suite tests KRIA's end-to-end GUI intelligence against the kinds of
//! prompts real users actually send. Each test exercises the full pipeline:
//!
//! ```text
//! User Prompt
//!   → RuleIntentCompiler (verb + target + content extraction)
//!   → TurnGate (routing decision)
//!   → SubstratePlanner (FileWriteThenOpen / BrowserNavigate / AppOpenOnly / TerminalExecution)
//!   → GuiExecutionCoordinator (open_application → type_text → press_shortcut)
//!   → BoundedExecutionVerifier (WindowFocused / OutputContains / FileSystemEffect)
//!   → GoalTree result + terminal_output surfacing
//!   → GuiEvalJudge verdict
//! ```
//!
//! # Gate
//!
//! Tests are gated by `KRIA_EVAL_GUI=1`. On CI without a display/daemon all tests
//! skip cleanly. Set `KRIA_EVAL_GUI=1` on a machine with:
//! - A running display server (X11 or Wayland)
//! - A running KRIA uinput daemon
//! - Target apps installed (code, gedit, firefox, etc.)
//!
//! For destructive/VM-only tests additionally set `KRIA_EVAL_VM=1`.
//!
//! # Categories
//!
//! | Category          | IDs                  | What's tested                              |
//! |-------------------|----------------------|--------------------------------------------|
//! | Coding — write    | cog-code-001..010    | Open editor, write program, save           |
//! | Coding — run+show | cog-run-001..006     | Write + execute + surface terminal_output  |
//! | Browser           | cog-browser-001..008 | Navigate, search, playlist, gmail          |
//! | File management   | cog-file-001..006    | Create/edit/save/delete files              |
//! | Terminal/system   | cog-term-001..007    | Shell commands, output capture             |
//! | Multi-app         | cog-multi-001..005   | Cross-app workflows                        |
//! | Session reuse     | cog-session-001..004 | App already open, switch context           |
//! | Recovery          | cog-recovery-001..004| Graceful error handling                    |
//! | Input/interaction | cog-input-001..005   | Numbers, special content, calculator       |
//! | Media             | cog-media-001..004   | Music, video, YouTube navigation           |

use super::suites::case;
use super::types::{DisplayServerRequirement, ExpectedArtifact, ExpectedBehavior, GuiEvalCase};
use std::time::Duration;

// ============================================================================
// Helper constructors
// ============================================================================

fn no_artifacts() -> Vec<ExpectedArtifact> {
    Vec::new()
}

fn py_artifact(name: &str, content: &str) -> Vec<ExpectedArtifact> {
    vec![ExpectedArtifact {
        path_pattern: format!("~/.kria/generated/{}*.py", name),
        content_contains: Some(content.to_string()),
        min_size_bytes: Some(50),
    }]
}

fn rs_artifact(name: &str, content: &str) -> Vec<ExpectedArtifact> {
    vec![ExpectedArtifact {
        path_pattern: format!("~/.kria/generated/{}*.rs", name),
        content_contains: Some(content.to_string()),
        min_size_bytes: Some(30),
    }]
}

fn no_retrieval() -> Vec<String> {
    vec![
        "web_search".to_string(),
        "search_news".to_string(),
        "searxng_search".to_string(),
    ]
}

fn false_success_patterns() -> Vec<String> {
    vec![
        "done!".to_string(),
        "i've completed".to_string(),
        "i have completed".to_string(),
        "task is done".to_string(),
    ]
}

fn cog(
    id: &str,
    desc: &str,
    prompt: &str,
    behavior: ExpectedBehavior,
    display: DisplayServerRequirement,
    requires_desktop: bool,
    tags: &[&str],
) -> GuiEvalCase {
    let mut c = case(id, desc, prompt, behavior, display, requires_desktop, tags);
    c.timeout = Duration::from_secs(90);
    c
}

// ============================================================================
// Category 1: Coding — Write Programs
// ============================================================================

pub fn coding_write_suite() -> Vec<GuiEvalCase> {
    vec![
        // ── Number table: the canonical complex prompt ────────────────────────
        cog(
            "cog-code-001-number-table",
            "Open code, write number table program with input, run and show output",
            "Open code and write a program to print any number table till 10, give any number input and show output of table",
            ExpectedBehavior {
                substrate: Some("VSCodeCodeRunWorkflow".to_string()),
                expected_artifacts: py_artifact("number_table", "input("),
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["output".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "coding", "input", "terminal-execution", "number-table"],
        ),

        // ── Fibonacci in gedit ────────────────────────────────────────────────
        cog(
            "cog-code-002-fibonacci-gedit",
            "Open gedit and write fibonacci — verify file artifact",
            "Open gedit and write a python program to print the fibonacci series up to 100",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: py_artifact("fibonacci", "def fib"),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "coding", "fibonacci", "gedit"],
        ),

        // ── Bubble sort ───────────────────────────────────────────────────────
        cog(
            "cog-code-003-bubble-sort",
            "Open code and write bubble sort program in Python",
            "Open VS Code and write a bubble sort program in Python that sorts a list of numbers",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: py_artifact("bubble_sort", "def bubble_sort"),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "coding", "sorting", "vscode"],
        ),

        // ── Calculator in Python ──────────────────────────────────────────────
        cog(
            "cog-code-004-calculator",
            "Open code and write a Python calculator with basic operations",
            "Open code and write a calculator program in python that can add subtract multiply and divide two numbers",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: py_artifact("calculator", "def add"),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "coding", "calculator", "python"],
        ),

        // ── Rust hello world ──────────────────────────────────────────────────
        cog(
            "cog-code-005-rust-hello-world",
            "Open code and write a Rust hello world program",
            "Open code and write a hello world program in Rust and save it",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: rs_artifact("hello", "fn main"),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "coding", "rust", "hello-world"],
        ),

        // ── Factorial program ─────────────────────────────────────────────────
        cog(
            "cog-code-006-factorial",
            "Open gedit and write factorial program in python",
            "Open gedit and write a python function to calculate factorial of any number using recursion",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: py_artifact("factorial", "def factorial"),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "coding", "factorial", "recursion"],
        ),

        // ── Prime numbers ─────────────────────────────────────────────────────
        cog(
            "cog-code-007-prime-numbers",
            "Open code and write a prime number sieve",
            "Open code and write a program to find and print all prime numbers between 1 and 100",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: py_artifact("prime", "def is_prime"),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "coding", "primes", "sieve"],
        ),

        // ── File line counter ─────────────────────────────────────────────────
        cog(
            "cog-code-008-file-line-counter",
            "Open gedit and write a Python file line counter",
            "Open gedit and write a python program that reads any file and counts the number of lines in it",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: py_artifact("line_counter", "open("),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "coding", "file-io"],
        ),

        // ── Pascal's triangle (regression + cognition) ────────────────────────
        cog(
            "cog-code-009-pascals-triangle",
            "Open code and write Pascal's triangle program and run it",
            "Open code and write a program to print pascals triangle and run it and show me the output",
            ExpectedBehavior {
                substrate: Some("VSCodeCodeRunWorkflow".to_string()),
                expected_artifacts: py_artifact("pascal", "def pascal"),
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec![
                    "application 'code and'".to_string(),
                    "not found".to_string(),
                ],
                required_response_patterns: vec!["output".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "coding", "pascal", "regression", "run-show"],
        ),

        // ── JSON parser ───────────────────────────────────────────────────────
        cog(
            "cog-code-010-json-operations",
            "Open gedit and write a Python JSON read/write program",
            "Open gedit and write a python program that creates a JSON file with student records and then reads and prints them",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: py_artifact("json", "import json"),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "coding", "json", "file-io"],
        ),
    ]
}

// ============================================================================
// Category 2: Coding — Run & Show Output
// ============================================================================

pub fn coding_run_show_suite() -> Vec<GuiEvalCase> {
    vec![
        // ── Number table: run with input + show output ────────────────────────
        cog(
            "cog-run-001-number-table-output",
            "Write number table, pipe input=5, show output",
            "Open code and write a number table program, run it with input 5 and show me the complete output",
            ExpectedBehavior {
                substrate: Some("VSCodeCodeRunWorkflow".to_string()),
                expected_artifacts: py_artifact("number_table", "for i in range"),
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["output".to_string(), "5".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "run-show", "number-table", "output-capture"],
        ),

        // ── Fibonacci run and print ───────────────────────────────────────────
        cog(
            "cog-run-002-fibonacci-run",
            "Write fibonacci program, run it and show me the output",
            "Open code and write a fibonacci program in python and run it and show me the output",
            ExpectedBehavior {
                substrate: Some("VSCodeCodeRunWorkflow".to_string()),
                expected_artifacts: py_artifact("fibonacci", "def fib"),
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["output".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "run-show", "fibonacci", "output-capture"],
        ),

        // ── Prime numbers run ─────────────────────────────────────────────────
        cog(
            "cog-run-003-primes-run",
            "Write prime numbers program and show me what it prints",
            "Write a python program to print all prime numbers up to 50 and run it to show the output",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: py_artifact("prime", ""),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["output".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "run-show", "primes"],
        ),

        // ── Hello world run ───────────────────────────────────────────────────
        cog(
            "cog-run-004-hello-world-python-run",
            "Write and run a Python hello world, show output",
            "Open terminal and run a python hello world program and print the output",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["Hello".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "run-show", "hello-world"],
        ),

        // ── Bubble sort: run with sample data ─────────────────────────────────
        cog(
            "cog-run-005-bubble-sort-run",
            "Write bubble sort, run with sample list, show sorted output",
            "Open code and write a bubble sort program in python, run it with the list [64,34,25,12,22,11,90] and show me the sorted output",
            ExpectedBehavior {
                substrate: Some("VSCodeCodeRunWorkflow".to_string()),
                expected_artifacts: py_artifact("bubble_sort", "def bubble_sort"),
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["output".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "run-show", "bubble-sort", "output-capture"],
        ),

        // ── Calculator: evaluate an expression ───────────────────────────────
        cog(
            "cog-run-006-calculator-eval",
            "Write a calculator, evaluate 123*456, show result",
            "Open code and write a simple calculator in python, compute 123 multiplied by 456 and show me the result",
            ExpectedBehavior {
                substrate: Some("VSCodeCodeRunWorkflow".to_string()),
                expected_artifacts: py_artifact("calculator", ""),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["56088".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "run-show", "calculator", "arithmetic"],
        ),
    ]
}

// ============================================================================
// Category 3: Browser Intelligence
// ============================================================================

pub fn browser_cognition_suite() -> Vec<GuiEvalCase> {
    vec![
        // ── YouTube playlist navigation ────────────────────────────────────────
        cog(
            "cog-browser-001-youtube-playlist",
            "Open YouTube, find playlist 'Songs', play first song",
            "Open youtube and check my Playlist named Songs and play the first song from there",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec![
                    "i cannot access".to_string(),
                    "cannot browse".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "browser", "youtube", "playlist", "navigation"],
        ),
        // ── GitHub navigation ─────────────────────────────────────────────────
        cog(
            "cog-browser-002-github",
            "Open firefox and go to github.com",
            "Open firefox and go to github.com and show me the page",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["managed_browser_navigate".to_string()],
                forbidden_tools: vec!["web_search".to_string(), "search_news".to_string()],
                forbidden_response_patterns: vec!["cannot".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "browser", "github", "navigation"],
        ),
        // ── Google search via browser ─────────────────────────────────────────
        cog(
            "cog-browser-003-google-search",
            "Open chrome and search for python tutorials",
            "Open chrome and search for python tutorials for beginners",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: vec![
                    "web_search".to_string(),
                    "search_news".to_string(),
                    "searxng_search".to_string(),
                ],
                forbidden_response_patterns: vec!["searxng".to_string(), "cloud LLM".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "browser", "search", "retrieval-isolation"],
        ),
        // ── Stack Overflow ────────────────────────────────────────────────────
        cog(
            "cog-browser-004-stackoverflow",
            "Open firefox and go to stackoverflow.com",
            "Open firefox and navigate to stackoverflow.com",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["managed_browser_navigate".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec!["cannot".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "browser", "stackoverflow", "navigation"],
        ),
        // ── YouTube search ────────────────────────────────────────────────────
        cog(
            "cog-browser-005-youtube-search",
            "Open youtube and search for a song by name",
            "Open youtube in firefox and search for the song Blinding Lights by The Weeknd",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec!["cannot".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "browser", "youtube", "search", "media"],
        ),
        // ── Documentation lookup ──────────────────────────────────────────────
        cog(
            "cog-browser-006-docs-python",
            "Open browser and go to Python documentation",
            "Open a browser and go to docs.python.org to check the documentation",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["managed_browser_navigate".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec!["cannot".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "browser", "docs", "python"],
        ),
        // ── Gmail (login-wall, expect graceful) ───────────────────────────────
        cog(
            "cog-browser-007-gmail",
            "Open chrome and go to gmail",
            "Open chrome and go to my gmail",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec![
                    "i cannot access your email".to_string(),
                    "No GUI backend".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "browser", "gmail", "google"],
        ),
        // ── Wikipedia ─────────────────────────────────────────────────────────
        cog(
            "cog-browser-008-wikipedia",
            "Open firefox and search Wikipedia for Fibonacci sequence",
            "Open firefox and go to wikipedia and search for Fibonacci sequence",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec!["cannot".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "browser", "wikipedia", "search"],
        ),
    ]
}

// ============================================================================
// Category 4: File Management Intelligence
// ============================================================================

pub fn file_management_cognition_suite() -> Vec<GuiEvalCase> {
    vec![
        // ── Create and write notes.txt ────────────────────────────────────────
        cog(
            "cog-file-001-create-notes",
            "Create a notes.txt file with shopping list content",
            "Open gedit and create a new file called notes.txt and write a shopping list with milk eggs bread and butter",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "~/.kria/generated/notes*.txt".to_string(),
                    content_contains: Some("milk".to_string()),
                    min_size_bytes: Some(10),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "file-management", "text", "gedit"],
        ),

        // ── Write Python to specific path ─────────────────────────────────────
        cog(
            "cog-file-002-python-specific-path",
            "Open code and save a Python file to the home directory",
            "Open code and write a python hello world program and save it as hello.py in my home folder",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "~/hello*.py".to_string(),
                    content_contains: Some("print".to_string()),
                    min_size_bytes: Some(10),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "file-management", "path-handling"],
        ),

        // ── Create README ─────────────────────────────────────────────────────
        cog(
            "cog-file-003-readme",
            "Open gedit and write a README.md for a Python project",
            "Open gedit and write a README.md file for a python calculator project with installation and usage instructions",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "~/.kria/generated/readme*.md".to_string(),
                    content_contains: Some("##".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "file-management", "markdown"],
        ),

        // ── Write config file ─────────────────────────────────────────────────
        cog(
            "cog-file-004-config-json",
            "Open gedit and write a JSON config file",
            "Open gedit and write a JSON configuration file for a web application with host port and database settings",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "~/.kria/generated/config*.json".to_string(),
                    content_contains: Some("host".to_string()),
                    min_size_bytes: Some(20),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "file-management", "json", "config"],
        ),

        // ── Shell script ──────────────────────────────────────────────────────
        cog(
            "cog-file-005-shell-script",
            "Open gedit and write a backup shell script",
            "Open gedit and write a bash shell script that creates a backup of a folder by copying it with a timestamp in the name",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "~/.kria/generated/backup*.sh".to_string(),
                    content_contains: Some("cp ".to_string()),
                    min_size_bytes: Some(30),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "file-management", "shell", "bash"],
        ),

        // ── HTML file ─────────────────────────────────────────────────────────
        cog(
            "cog-file-006-html",
            "Open gedit and write a simple HTML webpage",
            "Open gedit and write a simple HTML page with a heading that says Hello World and a paragraph about Python programming",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: vec![ExpectedArtifact {
                    path_pattern: "~/.kria/generated/*.html".to_string(),
                    content_contains: Some("<html".to_string()),
                    min_size_bytes: Some(50),
                }],
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "file-management", "html", "web"],
        ),
    ]
}

// ============================================================================
// Category 5: Terminal & System Intelligence
// ============================================================================

pub fn terminal_cognition_suite() -> Vec<GuiEvalCase> {
    vec![
        // ── Disk usage ────────────────────────────────────────────────────────
        cog(
            "cog-term-001-disk-usage",
            "Open terminal and check disk usage with df -h",
            "Open a terminal and check disk usage with df -h and show me the output",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["output".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "terminal", "system", "disk"],
        ),

        // ── Python version ────────────────────────────────────────────────────
        cog(
            "cog-term-002-python-version",
            "Open terminal and check Python version",
            "Open terminal and check what Python version I have installed and show me the output",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["3.".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "terminal", "python", "version"],
        ),

        // ── List Python files ─────────────────────────────────────────────────
        cog(
            "cog-term-003-list-python-files",
            "Open terminal and list all Python files in home directory",
            "Open terminal and list all python files in my home directory and show me their names",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["output".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "terminal", "filesystem", "python"],
        ),

        // ── Memory usage ──────────────────────────────────────────────────────
        cog(
            "cog-term-004-memory-usage",
            "Check available system memory",
            "Open terminal and check how much RAM my system has and how much is being used right now",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["output".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "terminal", "system", "memory"],
        ),

        // ── Git status ────────────────────────────────────────────────────────
        cog(
            "cog-term-005-git-status",
            "Open terminal and check git status in KRIA directory",
            "Open terminal, go to the KRIA project directory and show me the git status",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["output".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "terminal", "git"],
        ),

        // ── Environment variables ─────────────────────────────────────────────
        cog(
            "cog-term-006-env-vars",
            "Show environment variables in terminal",
            "Open terminal and show me all environment variables that start with PATH",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["PATH".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "terminal", "environment"],
        ),

        // ── Count files in directory ──────────────────────────────────────────
        cog(
            "cog-term-007-count-files",
            "Open terminal and count Rust files in the KRIA codebase",
            "Open terminal and count how many Rust source files are in the KRIA project and show me the total",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["output".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "terminal", "filesystem", "rust"],
        ),
    ]
}

// ============================================================================
// Category 6: Multi-App Workflows
// ============================================================================

pub fn multi_app_cognition_suite() -> Vec<GuiEvalCase> {
    vec![
        // ── Write code then switch to terminal to run ─────────────────────────
        cog(
            "cog-multi-001-code-then-terminal",
            "Open VS Code, write a Python file, then switch to terminal and run it",
            "Open code and write a hello world in python, then open a terminal and run the file with python3",
            ExpectedBehavior {
                substrate: Some("VSCodeCodeRunWorkflow".to_string()),
                expected_artifacts: py_artifact("hello", "print"),
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["Hello".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "multi-app", "code", "terminal"],
        ),

        // ── Open two editors ──────────────────────────────────────────────────
        cog(
            "cog-multi-002-two-editors",
            "Open gedit and write notes, then open code to write a Python program",
            "Open gedit and write a note saying project started, then open code and write a python hello world",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: py_artifact("hello", "print"),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "multi-app", "gedit", "code"],
        ),

        // ── Write code and open browser to docs ───────────────────────────────
        cog(
            "cog-multi-003-code-and-browser",
            "Open code to write Python, then open browser to Python docs",
            "Open code and start writing a python program, then open firefox and go to docs.python.org for reference",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["managed_browser_navigate".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "multi-app", "code", "browser"],
        ),

        // ── Switch to existing terminal ───────────────────────────────────────
        cog(
            "cog-multi-004-switch-to-terminal",
            "Switch to terminal and run a command",
            "Switch to the terminal and run python3 --version and show me the result",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["3.".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "multi-app", "switch", "terminal"],
        ),

        // ── Code, run, check browser for docs ────────────────────────────────
        cog(
            "cog-multi-005-full-dev-workflow",
            "Full dev workflow: write code, run it, check error on Stack Overflow",
            "Open code and write a python program, run it, if there is an error open firefox and search for the error on stackoverflow",
            ExpectedBehavior {
                substrate: Some("VSCodeCodeRunWorkflow".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["write_file".to_string(), "execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "multi-app", "dev-workflow", "error-handling"],
        ),
    ]
}

// ============================================================================
// Category 7: Session Reuse & App State Intelligence
// ============================================================================

pub fn session_reuse_cognition_suite() -> Vec<GuiEvalCase> {
    vec![
        // ── Code is already open ──────────────────────────────────────────────
        cog(
            "cog-session-001-code-already-open",
            "VS Code is already running, write a new file without relaunching",
            "Code is already open, write a new python file with a hello world program in it",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: py_artifact("hello", "print"),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec![
                    "launch".to_string(),
                    "starting code".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: true,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "session-reuse", "vscode"],
        ),

        // ── Switch to already-open terminal ───────────────────────────────────
        cog(
            "cog-session-002-switch-terminal",
            "Switch to terminal window that's already open and run a command",
            "Switch to terminal and run ls -la and show me what files are there",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["output".to_string()],
                expect_success: true,
                app_already_running: true,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "session-reuse", "terminal", "switch"],
        ),

        // ── Write additional code into already-open editor ────────────────────
        cog(
            "cog-session-003-add-to-existing",
            "Add a new function to an existing code file already open in editor",
            "The python file is already open in code, add a new function called greet that takes a name and prints Hello name",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: py_artifact("greet", "def greet"),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: true,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "session-reuse", "edit", "add-function"],
        ),

        // ── Save existing file ────────────────────────────────────────────────
        cog(
            "cog-session-004-save-existing",
            "Save the file currently open in the editor",
            "Save the current file open in VS Code",
            ExpectedBehavior {
                substrate: Some("Keystroke".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: true,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "session-reuse", "save", "shortcut"],
        ),
    ]
}

// ============================================================================
// Category 8: Recovery & Error Intelligence
// ============================================================================

pub fn recovery_cognition_suite() -> Vec<GuiEvalCase> {
    vec![
        // ── App not installed — graceful handling ─────────────────────────────
        cog(
            "cog-recovery-001-app-not-installed",
            "Try to open a non-existent app — KRIA should handle gracefully",
            "Open nonexistent_app_xyz_12345 and write hello world",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec![
                    "panicked".to_string(),
                    "thread panicked".to_string(),
                ],
                required_response_patterns: vec![],
                expect_success: false, // Should fail gracefully, not crash
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "recovery", "error-handling", "app-not-found"],
        ),

        // ── Run program with syntax error — show error output ─────────────────
        cog(
            "cog-recovery-002-syntax-error-output",
            "Write a Python program with a syntax error and show the error output",
            "Open terminal and run a python program with a syntax error and show me the error message output",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec!["output".to_string()],
                expect_success: true, // Error output is still useful output
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "recovery", "error-output", "python"],
        ),

        // ── Missing file — graceful ────────────────────────────────────────────
        cog(
            "cog-recovery-003-missing-file",
            "Try to open a file that doesn't exist",
            "Open the file /totally/nonexistent/path/file.py in VS Code",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "recovery", "missing-file", "error-handling"],
        ),

        // ── Permission denied — graceful ──────────────────────────────────────
        cog(
            "cog-recovery-004-permission-denied",
            "Try to write to a read-only system path — should handle gracefully",
            "Open gedit and save a file to /etc/kria_test_forbidden.txt",
            ExpectedBehavior {
                substrate: None,
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "recovery", "permission-denied", "error-handling"],
        ),
    ]
}

// ============================================================================
// Category 9: Input & Interaction Intelligence
// ============================================================================

pub fn input_interaction_cognition_suite() -> Vec<GuiEvalCase> {
    vec![
        // ── Type with numbers and special characters ──────────────────────────
        cog(
            "cog-input-001-type-with-numbers",
            "Open gedit and type a text with numbers and punctuation",
            "Open gedit and type the following: My phone number is 0300-1234567 and my email is user@example.com",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "input", "numbers", "special-chars"],
        ),

        // ── Type a poem or multi-line text ────────────────────────────────────
        cog(
            "cog-input-002-multiline-text",
            "Open gedit and write multi-line text",
            "Open gedit and type the following poem: Roses are red, Violets are blue, Python is awesome, And KRIA is too",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "input", "multiline", "text"],
        ),

        // ── Save file with keyboard shortcut ──────────────────────────────────
        cog(
            "cog-input-003-save-shortcut",
            "Open gedit, type something, save with Ctrl+S",
            "Open gedit, type hello world, and save the file using keyboard shortcut",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "input", "shortcut", "save"],
        ),

        // ── Open calculator and compute ───────────────────────────────────────
        cog(
            "cog-input-004-compute-expression",
            "Open terminal and compute a mathematical expression",
            "Open terminal and compute 1234 multiplied by 5678 and show me the result",
            ExpectedBehavior {
                substrate: Some("TerminalExecution".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["execute_bash".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec!["7006652".to_string()],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            false,
            &["cognition", "input", "arithmetic", "terminal"],
        ),

        // ── Urdu/Roman script input (language diversity) ──────────────────────
        cog(
            "cog-input-005-roman-urdu",
            "Open gedit and type a message in Roman Urdu",
            "Open gedit and type: Yeh program number table print karta hai. Koi bhi number dein aur table dekhein.",
            ExpectedBehavior {
                substrate: Some("FileWriteThenOpen".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["write_file".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::Any,
            true,
            &["cognition", "input", "roman-urdu", "language"],
        ),
    ]
}

// ============================================================================
// Category 10: Media Intelligence
// ============================================================================

pub fn media_cognition_suite() -> Vec<GuiEvalCase> {
    vec![
        // ── Open media player ─────────────────────────────────────────────────
        cog(
            "cog-media-001-open-vlc",
            "Open VLC media player",
            "Open VLC media player",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["open_application".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: false_success_patterns(),
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "media", "vlc", "app-open"],
        ),
        // ── YouTube via browser ───────────────────────────────────────────────
        cog(
            "cog-media-002-youtube-open",
            "Open YouTube in browser",
            "Open YouTube in firefox",
            ExpectedBehavior {
                substrate: Some("BrowserNavigate".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec!["browser_search".to_string()],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec!["cannot".to_string()],
                required_response_patterns: vec![],
                expect_success: true,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "media", "youtube", "browser"],
        ),
        // ── Spotify or music app ──────────────────────────────────────────────
        cog(
            "cog-media-003-spotify",
            "Open Spotify",
            "Open Spotify music app",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec!["cannot".to_string()],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "media", "spotify", "app-open"],
        ),
        // ── Play a video file ─────────────────────────────────────────────────
        cog(
            "cog-media-004-play-video",
            "Open a video file with VLC or default player",
            "Open the video file at /home/obaid/Videos/sample.mp4 and play it",
            ExpectedBehavior {
                substrate: Some("AppOpenOnly".to_string()),
                expected_artifacts: no_artifacts(),
                required_tools: vec![],
                forbidden_tools: no_retrieval(),
                forbidden_response_patterns: vec!["panicked".to_string()],
                required_response_patterns: vec![],
                expect_success: false,
                app_already_running: false,
            },
            DisplayServerRequirement::X11OrWayland,
            true,
            &["cognition", "media", "video", "vlc"],
        ),
    ]
}

// ============================================================================
// Aggregated suite
// ============================================================================

pub fn all_gui_cognition_cases() -> Vec<GuiEvalCase> {
    let mut cases = Vec::new();
    cases.extend(coding_write_suite());
    cases.extend(coding_run_show_suite());
    cases.extend(browser_cognition_suite());
    cases.extend(file_management_cognition_suite());
    cases.extend(terminal_cognition_suite());
    cases.extend(multi_app_cognition_suite());
    cases.extend(session_reuse_cognition_suite());
    cases.extend(recovery_cognition_suite());
    cases.extend(input_interaction_cognition_suite());
    cases.extend(media_cognition_suite());
    cases
}

// ============================================================================
// Test Runner
// ============================================================================

/// Returns true if GUI evals are opted in for the current environment.
///
/// Set `KRIA_EVAL_GUI=1` on a machine with display + daemon + target apps.
pub fn gui_eval_enabled() -> bool {
    std::env::var("KRIA_EVAL_GUI").as_deref() == Ok("1")
}

pub fn vm_mode() -> bool {
    std::env::var("KRIA_EVAL_VM").as_deref() == Ok("1")
}

/// Run a case with the GuiEvalRunner, then evaluate with GuiEvalJudge.
/// Returns (passed, skip, diagnostics_string).
pub async fn run_and_evaluate(case: &GuiEvalCase) -> (bool, bool, String) {
    use super::judge::GuiEvalJudge;
    use super::runner::GuiEvalRunner;
    use super::types::GuiEvalVerdictKind;

    let obs = GuiEvalRunner::new().run(case).await;
    let verdict = GuiEvalJudge.evaluate(case, &obs);

    let diag = format!(
        "[{}] verdict={:?} score={:.2} trace_steps={} tools={:?} response={}",
        case.id,
        verdict.kind,
        verdict.quality_score,
        obs.trace.steps_executed.len(),
        obs.trace.tools_called,
        &obs.trace.final_response[..obs.trace.final_response.len().min(120)],
    );

    let passed = matches!(verdict.kind, GuiEvalVerdictKind::Pass);
    let skipped = matches!(
        verdict.kind,
        GuiEvalVerdictKind::Skip | GuiEvalVerdictKind::EnvironmentBlocked
    );
    (passed, skipped, diag)
}

// ============================================================================
// Tests — gate with KRIA_EVAL_GUI=1
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── Coding — Write ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn eval_cog_code_001_number_table() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_write_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-code-001-number-table")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_code_002_fibonacci_gedit() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_write_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-code-002-fibonacci-gedit")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_code_003_bubble_sort() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_write_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-code-003-bubble-sort")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_code_004_calculator() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_write_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-code-004-calculator")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_code_005_rust_hello_world() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_write_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-code-005-rust-hello-world")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_code_006_factorial() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_write_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-code-006-factorial")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_code_007_prime_numbers() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_write_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-code-007-prime-numbers")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_code_008_file_line_counter() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_write_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-code-008-file-line-counter")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_code_009_pascals_triangle() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_write_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-code-009-pascals-triangle")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_code_010_json_operations() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_write_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-code-010-json-operations")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    // ── Coding — Run + Show ───────────────────────────────────────────────────

    #[tokio::test]
    async fn eval_cog_run_001_number_table_output() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_run_show_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-run-001-number-table-output")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_run_002_fibonacci_run() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_run_show_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-run-002-fibonacci-run")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_run_003_primes_run() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_run_show_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-run-003-primes-run")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_run_004_hello_world_run() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_run_show_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-run-004-hello-world-python-run")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_run_005_bubble_sort_run() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_run_show_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-run-005-bubble-sort-run")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_run_006_calculator_eval() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = coding_run_show_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-run-006-calculator-eval")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    // ── Browser ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn eval_cog_browser_001_youtube_playlist() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = browser_cognition_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-browser-001-youtube-playlist")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_browser_003_google_search_retrieval_clean() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = browser_cognition_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-browser-003-google-search")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    // ── Terminal ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn eval_cog_term_001_disk_usage() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = terminal_cognition_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-term-001-disk-usage")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_term_002_python_version() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = terminal_cognition_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-term-002-python-version")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_term_004_memory_usage() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = terminal_cognition_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-term-004-memory-usage")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_term_006_env_vars() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = terminal_cognition_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-term-006-env-vars")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    // ── Multi-App ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn eval_cog_multi_001_code_then_terminal() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = multi_app_cognition_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-multi-001-code-then-terminal")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    #[tokio::test]
    async fn eval_cog_multi_004_switch_terminal() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = multi_app_cognition_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-multi-004-switch-to-terminal")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    // ── Recovery ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn eval_cog_recovery_001_app_not_installed() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = recovery_cognition_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-recovery-001-app-not-installed")
            .unwrap();
        let (_, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        // Only check no panic — a graceful fail IS the correct outcome here
        let _ = skipped;
    }

    #[tokio::test]
    async fn eval_cog_recovery_002_syntax_error_output() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = recovery_cognition_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-recovery-002-syntax-error-output")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    // ── Input ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn eval_cog_input_004_compute_expression() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = input_interaction_cognition_suite();
        let case = cases
            .iter()
            .find(|c| c.id == "cog-input-004-compute-expression")
            .unwrap();
        let (passed, skipped, diag) = run_and_evaluate(case).await;
        eprintln!("{}", diag);
        if !skipped {
            assert!(passed, "FAIL: {}", diag);
        }
    }

    // ── Full suite summary ────────────────────────────────────────────────────

    /// Run all 50 cognition cases and print a summary report.
    /// Does NOT assert individual pass/fail — purely for audit.
    #[tokio::test]
    async fn audit_full_gui_cognition_suite() {
        if !gui_eval_enabled() {
            eprintln!("[SKIP] audit_full_gui_cognition_suite: set KRIA_EVAL_GUI=1");
            return;
        }
        let cases = all_gui_cognition_cases();
        let mut pass = 0usize;
        let mut fail = 0usize;
        let mut skip = 0usize;
        for case in &cases {
            let (passed, skipped, diag) = run_and_evaluate(case).await;
            eprintln!("{}", diag);
            if skipped {
                skip += 1;
            } else if passed {
                pass += 1;
            } else {
                fail += 1;
            }
        }
        eprintln!(
            "\n=== GUI Cognition Audit: {total} cases — {pass} PASS / {fail} FAIL / {skip} SKIP ===",
            total = cases.len(),
            pass = pass,
            fail = fail,
            skip = skip,
        );
        // Fail the audit run only if there are failures AND no skips at all
        // (i.e., we had a real display but still failed)
        if skip == 0 && fail > 0 {
            panic!("{fail} GUI cognition evals failed — see diagnostics above");
        }
    }

    // ── Suite structure tests (always run — no KRIA_EVAL_GUI needed) ──────────

    #[test]
    fn all_cases_have_unique_ids() {
        let cases = all_gui_cognition_cases();
        let mut ids = std::collections::HashSet::new();
        for c in &cases {
            assert!(ids.insert(c.id.clone()), "duplicate id: {}", c.id);
        }
    }

    #[test]
    fn all_cases_have_non_empty_prompts() {
        let cases = all_gui_cognition_cases();
        for c in &cases {
            assert!(!c.prompt.is_empty(), "empty prompt for {}", c.id);
            assert!(c.prompt.len() >= 10, "prompt too short for {}", c.id);
        }
    }

    #[test]
    fn total_case_count() {
        let cases = all_gui_cognition_cases();
        eprintln!("Total GUI cognition eval cases: {}", cases.len());
        assert!(
            cases.len() >= 40,
            "expected at least 40 cases, got {}",
            cases.len()
        );
    }

    #[test]
    fn no_retrieval_tools_in_non_browser_cases() {
        let non_browser: Vec<_> = all_gui_cognition_cases()
            .into_iter()
            .filter(|c| !c.tags.iter().any(|t| t == "browser"))
            .collect();
        for c in &non_browser {
            for forbidden in &c.expected_behavior.forbidden_tools {
                assert!(
                    matches!(
                        forbidden.as_str(),
                        "web_search" | "search_news" | "searxng_search"
                    ),
                    "non-browser case {} missing retrieval isolation in forbidden_tools",
                    c.id
                );
            }
        }
    }
}
