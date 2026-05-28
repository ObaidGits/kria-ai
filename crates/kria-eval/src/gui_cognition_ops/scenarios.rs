//! Realistic GUI Cognition Eval Scenarios
//!
//! Each scenario represents a REAL human workflow that KRIA must handle.
//! These are NOT toy prompts — they test multistep cognition, app coordination,
//! focus management, verification, and recovery.

use serde::{Deserialize, Serialize};

/// A single eval scenario — a real workflow KRIA must execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalScenario {
    /// Unique scenario identifier
    pub id: String,
    /// Human-readable description
    pub description: String,
    /// The actual prompt to send to KRIA
    pub prompt: String,
    /// Category for classification
    pub category: EvalCategory,
    /// Expected minimum steps
    pub expected_min_steps: u32,
    /// Maximum allowed duration (seconds)
    pub max_duration_secs: u64,
    /// What constitutes success
    pub success_criteria: Vec<SuccessCriterion>,
    /// Known failure modes to watch for
    pub known_risks: Vec<String>,
    /// Difficulty level
    pub difficulty: Difficulty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvalCategory {
    IdeWorkflow,
    BrowserWorkflow,
    FileManagement,
    InteractiveGui,
    LoginInstall,
    FocusStealing,
    Recovery,
    Cancellation,
    LongHorizon,
    MultiApp,
    Interruption,
    EnvironmentInstability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Difficulty {
    Basic,
    Intermediate,
    Advanced,
    Stress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuccessCriterion {
    /// A file must exist at this path
    FileExists(String),
    /// A process must be running
    ProcessRunning(String),
    /// Browser must have navigated (any evidence)
    BrowserNavigated,
    /// App window must be visible
    AppVisible(String),
    /// Output file must contain substring
    OutputContains { file: String, substring: String },
    /// Port must be listening
    PortListening(u16),
    /// Workflow must complete without timeout
    NoTimeout,
    /// HITL must be triggered (for login/install scenarios)
    HitlTriggered,
    /// Verdict must not be Failed
    VerdictNotFailed,
}

/// Get all registered eval scenarios.
pub fn all_scenarios() -> Vec<EvalScenario> {
    let mut scenarios = Vec::new();
    scenarios.extend(ide_scenarios());
    scenarios.extend(browser_scenarios());
    scenarios.extend(file_scenarios());
    scenarios.extend(interactive_scenarios());
    scenarios.extend(recovery_scenarios());
    scenarios.extend(multi_app_scenarios());
    scenarios.extend(long_horizon_scenarios());
    scenarios
}

// ═══════════════════════════════════════════════════════════════════════════════
// IDE + Development Workflows
// ═══════════════════════════════════════════════════════════════════════════════

fn ide_scenarios() -> Vec<EvalScenario> {
    vec![
        EvalScenario {
            id: "ide-001-python-hello".into(),
            description: "Generate and run a simple Python script in VS Code".into(),
            prompt: "Open Code, create a Python file that prints 'Hello KRIA', run it, and show me the output.".into(),
            category: EvalCategory::IdeWorkflow,
            expected_min_steps: 3,
            max_duration_secs: 60,
            success_criteria: vec![
                SuccessCriterion::FileExists("/home/obaid/.kria/generated/".into()),
                SuccessCriterion::ProcessRunning("code".into()),
                SuccessCriterion::NoTimeout,
            ],
            known_risks: vec!["VS Code startup delay".into(), "terminal focus drift".into()],
            difficulty: Difficulty::Basic,
        },
        EvalScenario {
            id: "ide-002-react-project".into(),
            description: "Generate a React website project and run dev server".into(),
            prompt: "Open Code and generate a website for a Web Development Agency, install dependencies, run the dev server, and show me the result in the browser.".into(),
            category: EvalCategory::IdeWorkflow,
            expected_min_steps: 5,
            max_duration_secs: 120,
            success_criteria: vec![
                SuccessCriterion::ProcessRunning("node".into()),
                SuccessCriterion::PortListening(3000),
                SuccessCriterion::BrowserNavigated,
                SuccessCriterion::NoTimeout,
            ],
            known_risks: vec!["npm install timeout".into(), "port conflict".into(), "browser launch race".into()],
            difficulty: Difficulty::Advanced,
        },
        EvalScenario {
            id: "ide-003-python-scraper".into(),
            description: "Generate a Python web scraper, execute it, save output".into(),
            prompt: "Open Code, generate a Python script that fetches the title of https://example.com, run it, and save the output to a file.".into(),
            category: EvalCategory::IdeWorkflow,
            expected_min_steps: 4,
            max_duration_secs: 45,
            success_criteria: vec![
                SuccessCriterion::NoTimeout,
                SuccessCriterion::VerdictNotFailed,
            ],
            known_risks: vec!["network timeout".into(), "missing requests library".into()],
            difficulty: Difficulty::Intermediate,
        },
        EvalScenario {
            id: "ide-004-rust-compile".into(),
            description: "Create a Rust file with intentional error, fix it, recompile".into(),
            prompt: "Create a Rust program that prints fibonacci numbers, run it with cargo, and show me the output.".into(),
            category: EvalCategory::IdeWorkflow,
            expected_min_steps: 3,
            max_duration_secs: 90,
            success_criteria: vec![
                SuccessCriterion::NoTimeout,
                SuccessCriterion::VerdictNotFailed,
            ],
            known_risks: vec!["cargo compile time".into(), "missing toolchain".into()],
            difficulty: Difficulty::Intermediate,
        },
    ]
}

// ═══════════════════════════════════════════════════════════════════════════════
// Browser Workflows
// ═══════════════════════════════════════════════════════════════════════════════

fn browser_scenarios() -> Vec<EvalScenario> {
    vec![
        EvalScenario {
            id: "browser-001-navigate-url".into(),
            description: "Open browser and navigate to a specific URL".into(),
            prompt: "Open the browser and go to https://example.com. Show me that the page loaded.".into(),
            category: EvalCategory::BrowserWorkflow,
            expected_min_steps: 1,
            max_duration_secs: 30,
            success_criteria: vec![
                SuccessCriterion::BrowserNavigated,
                SuccessCriterion::NoTimeout,
            ],
            known_risks: vec!["CDP timeout".into(), "browser launch race".into(), "xdg-open fallback".into()],
            difficulty: Difficulty::Basic,
        },
        EvalScenario {
            id: "browser-002-search-youtube".into(),
            description: "Open browser and search YouTube".into(),
            prompt: "Open Chrome and search for 'lofi music' on YouTube.".into(),
            category: EvalCategory::BrowserWorkflow,
            expected_min_steps: 1,
            max_duration_secs: 30,
            success_criteria: vec![
                SuccessCriterion::BrowserNavigated,
                SuccessCriterion::NoTimeout,
            ],
            known_risks: vec!["YouTube login wall".into(), "search redirect".into()],
            difficulty: Difficulty::Basic,
        },
        EvalScenario {
            id: "browser-003-localhost".into(),
            description: "Open browser to localhost after starting a server".into(),
            prompt: "Start a simple Python HTTP server on port 8080 and open it in the browser.".into(),
            category: EvalCategory::BrowserWorkflow,
            expected_min_steps: 2,
            max_duration_secs: 30,
            success_criteria: vec![
                SuccessCriterion::PortListening(8080),
                SuccessCriterion::BrowserNavigated,
                SuccessCriterion::NoTimeout,
            ],
            known_risks: vec!["port already in use".into(), "server startup race".into()],
            difficulty: Difficulty::Intermediate,
        },
        EvalScenario {
            id: "browser-004-outbro".into(),
            description: "Navigate to outbro.net and verify page load".into(),
            prompt: "Open the browser and go to https://outbro.net Show me that the page loaded.".into(),
            category: EvalCategory::BrowserWorkflow,
            expected_min_steps: 1,
            max_duration_secs: 30,
            success_criteria: vec![
                SuccessCriterion::BrowserNavigated,
                SuccessCriterion::NoTimeout,
            ],
            known_risks: vec!["CDP unavailable".into(), "managed browser timeout".into()],
            difficulty: Difficulty::Basic,
        },
    ]
}

// ═══════════════════════════════════════════════════════════════════════════════
// File Management Workflows
// ═══════════════════════════════════════════════════════════════════════════════

fn file_scenarios() -> Vec<EvalScenario> {
    vec![
        EvalScenario {
            id: "file-001-open-downloads".into(),
            description: "Open file manager to Downloads folder".into(),
            prompt: "Open the file manager to my Downloads folder and show me what's there.".into(),
            category: EvalCategory::FileManagement,
            expected_min_steps: 1,
            max_duration_secs: 15,
            success_criteria: vec![
                SuccessCriterion::ProcessRunning("nautilus".into()),
                SuccessCriterion::NoTimeout,
            ],
            known_risks: vec!["nautilus not installed".into(), "file manager variant".into()],
            difficulty: Difficulty::Basic,
        },
        EvalScenario {
            id: "file-002-create-project-structure".into(),
            description: "Create a project folder structure".into(),
            prompt: "Create a project folder called 'my-app' in /tmp with src, tests, and docs subfolders, and a README.md file.".into(),
            category: EvalCategory::FileManagement,
            expected_min_steps: 2,
            max_duration_secs: 15,
            success_criteria: vec![
                SuccessCriterion::FileExists("/tmp/my-app/README.md".into()),
                SuccessCriterion::NoTimeout,
            ],
            known_risks: vec!["permission issues".into()],
            difficulty: Difficulty::Basic,
        },
    ]
}

// ═══════════════════════════════════════════════════════════════════════════════
// Interactive GUI Workflows
// ═══════════════════════════════════════════════════════════════════════════════

fn interactive_scenarios() -> Vec<EvalScenario> {
    vec![
        EvalScenario {
            id: "interactive-001-text-editor-draft".into(),
            description: "Open text editor and draft an email".into(),
            prompt: "Open a text editor and draft a short email saying 'Thank you for the meeting today'. Show me the draft for approval.".into(),
            category: EvalCategory::InteractiveGui,
            expected_min_steps: 2,
            max_duration_secs: 20,
            success_criteria: vec![
                SuccessCriterion::NoTimeout,
                SuccessCriterion::VerdictNotFailed,
            ],
            known_risks: vec!["editor variant".into(), "typing safety".into()],
            difficulty: Difficulty::Intermediate,
        },
        EvalScenario {
            id: "interactive-002-spreadsheet".into(),
            description: "Open spreadsheet and create columns".into(),
            prompt: "Open a spreadsheet application and create a sheet with columns: Item, Quantity, Price, Total.".into(),
            category: EvalCategory::InteractiveGui,
            expected_min_steps: 2,
            max_duration_secs: 30,
            success_criteria: vec![
                SuccessCriterion::NoTimeout,
                SuccessCriterion::VerdictNotFailed,
            ],
            known_risks: vec!["LibreOffice not installed".into(), "app alias resolution".into()],
            difficulty: Difficulty::Intermediate,
        },
    ]
}

// ═══════════════════════════════════════════════════════════════════════════════
// Recovery + Failure Workflows
// ═══════════════════════════════════════════════════════════════════════════════

fn recovery_scenarios() -> Vec<EvalScenario> {
    vec![
        EvalScenario {
            id: "recovery-001-missing-app".into(),
            description: "Handle missing application gracefully".into(),
            prompt: "Open Blender and create a 3D model.".into(),
            category: EvalCategory::Recovery,
            expected_min_steps: 1,
            max_duration_secs: 10,
            success_criteria: vec![
                SuccessCriterion::HitlTriggered,
            ],
            known_risks: vec!["silent failure instead of HITL".into()],
            difficulty: Difficulty::Basic,
        },
        EvalScenario {
            id: "recovery-002-invalid-command".into(),
            description: "Handle invalid command execution gracefully".into(),
            prompt: "Run the command 'nonexistent_tool_xyz --version' and show me the output.".into(),
            category: EvalCategory::Recovery,
            expected_min_steps: 1,
            max_duration_secs: 15,
            success_criteria: vec![
                SuccessCriterion::NoTimeout,
            ],
            known_risks: vec!["silent swallow of error".into()],
            difficulty: Difficulty::Basic,
        },
    ]
}

// ═══════════════════════════════════════════════════════════════════════════════
// Multi-App Workflows
// ═══════════════════════════════════════════════════════════════════════════════

fn multi_app_scenarios() -> Vec<EvalScenario> {
    vec![
        EvalScenario {
            id: "multi-001-code-and-browser".into(),
            description: "VS Code + Browser coordination".into(),
            prompt: "Open Code, create an HTML file with a hello world page, then open it in the browser.".into(),
            category: EvalCategory::MultiApp,
            expected_min_steps: 3,
            max_duration_secs: 45,
            success_criteria: vec![
                SuccessCriterion::ProcessRunning("code".into()),
                SuccessCriterion::BrowserNavigated,
                SuccessCriterion::NoTimeout,
            ],
            known_risks: vec!["focus switching".into(), "file path resolution".into()],
            difficulty: Difficulty::Intermediate,
        },
    ]
}

// ═══════════════════════════════════════════════════════════════════════════════
// Long-Horizon Workflows
// ═══════════════════════════════════════════════════════════════════════════════

fn long_horizon_scenarios() -> Vec<EvalScenario> {
    vec![
        EvalScenario {
            id: "long-001-full-dev-cycle".into(),
            description: "Complete development cycle: create, code, test, run, verify".into(),
            prompt: "Create a Python project folder, write a calculator module with add/subtract/multiply functions, write unit tests, run the tests, and show me the test results.".into(),
            category: EvalCategory::LongHorizon,
            expected_min_steps: 5,
            max_duration_secs: 90,
            success_criteria: vec![
                SuccessCriterion::NoTimeout,
                SuccessCriterion::VerdictNotFailed,
            ],
            known_risks: vec!["multistep drift".into(), "context loss".into()],
            difficulty: Difficulty::Advanced,
        },
    ]
}
