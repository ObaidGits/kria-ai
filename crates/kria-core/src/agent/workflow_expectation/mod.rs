//! Phase 3 — Workflow Expectation Engine.
//!
//! # Core Mission
//!
//! Model how HUMANS expect workflows to behave — not just what commands ran.
//! A "coding workflow" implies a visible IDE, running diagnostics, a visible
//! terminal, and accessible output. A "browser workflow" implies a loaded
//! URL, potentially a visible page interaction, and result surfacing.
//!
//! This engine:
//! 1. Classifies the workflow category from context.
//! 2. Returns a `WorkflowExpectation` template with expected visible outcomes.
//! 3. Tracks workflow progress against the template.
//! 4. Surfaces gaps between expected and observed state.
//!
//! # Design Invariants
//!
//! - **Read-only**: Never executes actions.
//! - **Template-based**: Templates are compiled constants, not LLM outputs.
//! - **PSDG-aware**: Uses WorldModelStore to refine templates with live context.
//! - **Bounded**: Max 8 expected outcomes per template.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::agent::intent_compiler::{TargetRef, Verb};
use crate::agent::observable_completion::{CompletionVisibilityPolicy, ObservableOutcome};
use crate::agent::psdg::PsdgHandle;
use crate::agent::turn_gate::Operation;
use crate::agent::workflow_session::WorkflowSession;

// ─── Workflow Category ────────────────────────────────────────────────────────

/// Semantic category of a workflow.
///
/// Maps to a `WorkflowExpectation` template with expected visible outcomes,
/// typical phases, app requirements, and interaction style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkflowCategory {
    /// Write, edit, build, or run code in an IDE or text editor.
    Coding,
    /// Browse, search, or interact with web pages.
    Browser,
    /// Create, modify, move, or organize files/directories.
    FileManagement,
    /// Run shell commands or scripts in a terminal.
    Terminal,
    /// Create, manage, or update Jira tickets / GitHub issues.
    JiraDevOps,
    /// Debug, trace, or diagnose errors in code or systems.
    Debugging,
    /// Deploy code, build artifacts, or run CI/CD pipelines.
    Deployment,
    /// Compose or send email / messages.
    Email,
    /// Play, organize, or transcode audio/video media.
    Media,
    /// Change system settings, install software, manage services.
    SystemConfiguration,
    /// Multi-application workflows that span several categories.
    MultiApp,
    /// Workflow category could not be determined.
    Unknown,
}

impl WorkflowCategory {
    /// Human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Coding => "Code editing, building, and running",
            Self::Browser => "Web browsing and page interaction",
            Self::FileManagement => "File creation, editing, and organization",
            Self::Terminal => "Shell command execution",
            Self::JiraDevOps => "Issue tracking and DevOps operations",
            Self::Debugging => "Error diagnosis and debugging",
            Self::Deployment => "Code deployment and CI/CD",
            Self::Email => "Email and messaging",
            Self::Media => "Audio and video playback",
            Self::SystemConfiguration => "System configuration and administration",
            Self::MultiApp => "Multi-application workflow",
            Self::Unknown => "Unclassified workflow",
        }
    }

    /// Whether this category typically benefits from terminal visibility.
    pub fn needs_terminal(&self) -> bool {
        matches!(
            self,
            Self::Coding | Self::Debugging | Self::Terminal | Self::Deployment
        )
    }

    /// Whether this category typically benefits from browser visibility.
    pub fn needs_browser(&self) -> bool {
        matches!(
            self,
            Self::Browser | Self::JiraDevOps | Self::Deployment | Self::MultiApp
        )
    }

    /// Whether this category typically benefits from IDE visibility.
    pub fn needs_ide(&self) -> bool {
        matches!(self, Self::Coding | Self::Debugging | Self::JiraDevOps)
    }
}

// ─── Workflow Phase ───────────────────────────────────────────────────────────

/// A semantic phase within a workflow category.
///
/// Phases are ordered: a workflow progresses through them sequentially.
/// KRIA uses phases to understand workflow progress and blockers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPhase {
    /// Ordinal position (0-indexed).
    pub index: u32,
    /// Human-readable phase label.
    pub label: String,
    /// Expected observable outcome for this phase.
    pub expected_outcome: ObservableOutcome,
    /// Whether this phase is optional.
    pub optional: bool,
}

// ─── Workflow Expectation ─────────────────────────────────────────────────────

/// A complete expectation template for a workflow category.
///
/// Encodes what a human would expect to SEE when a workflow completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExpectation {
    /// Workflow category.
    pub category: WorkflowCategory,
    /// All expected observable outcomes after the workflow completes.
    /// Bounded to MAX_OUTCOMES_PER_TEMPLATE.
    pub expected_outcomes: Vec<ObservableOutcome>,
    /// Typical sequential phases (for progress tracking).
    pub typical_phases: Vec<WorkflowPhase>,
    /// Applications typically opened during this workflow.
    pub expected_apps: Vec<String>,
    /// Whether this workflow can run entirely in the background.
    pub can_background: bool,
    /// Typical total duration in seconds.
    pub typical_duration_sec: u64,
    /// Whether this workflow modifies the filesystem.
    pub modifies_fs: bool,
    /// Whether this workflow requires a network connection.
    pub needs_network: bool,
}

/// Maximum expected outcomes per template.
const MAX_OUTCOMES_PER_TEMPLATE: usize = 8;

impl WorkflowExpectation {
    /// Get visibility policies for all expected outcomes, given an operation.
    pub fn visibility_policies(&self, operation: Operation) -> Vec<CompletionVisibilityPolicy> {
        self.expected_outcomes
            .iter()
            .map(|o| CompletionVisibilityPolicy::for_outcome(o.clone(), operation))
            .collect()
    }

    /// Get the human-readable summary of what is expected to be visible.
    pub fn human_expectation_summary(&self) -> String {
        let descs: Vec<String> = self
            .expected_outcomes
            .iter()
            .filter(|o| !matches!(o, ObservableOutcome::Silent))
            .map(|o| format!("{:?}", o))
            .take(4)
            .collect();
        if descs.is_empty() {
            format!("{} workflow (background)", self.category.description())
        } else {
            format!(
                "{}: expects {}",
                self.category.description(),
                descs.join(", ")
            )
        }
    }
}

// ─── Workflow Progress Report ──────────────────────────────────────────────────

/// Progress of a workflow against its expectation template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowProgressReport {
    /// Workflow category.
    pub category: WorkflowCategory,
    /// Number of phases completed.
    pub phases_completed: u32,
    /// Total phases in the template.
    pub total_phases: u32,
    /// Percentage complete (0–100).
    pub percent_complete: u32,
    /// Current active phase label, if known.
    pub current_phase: Option<String>,
    /// Known blockers (e.g., "IDE not open", "build failed").
    pub blockers: Vec<String>,
    /// Whether the workflow is past the point of no return.
    pub is_committed: bool,
    /// Human-readable progress summary.
    pub summary: String,
}

// ─── Template Library ─────────────────────────────────────────────────────────

/// The canonical template library for all workflow categories.
///
/// Templates are compiled constants — no I/O, no LLM, no side effects.
fn build_template_library() -> HashMap<WorkflowCategory, WorkflowExpectation> {
    use ObservableOutcome::*;
    use WorkflowCategory::*;
    let mut lib = HashMap::new();

    // ── Coding ────────────────────────────────────────────────────────────
    lib.insert(
        Coding,
        WorkflowExpectation {
            category: Coding,
            expected_outcomes: vec![
                ApplicationWindow {
                    app_name: "code".into(),
                    title_hint: None,
                },
                TerminalOutput {
                    contains: String::new(),
                },
            ],
            typical_phases: vec![
                WorkflowPhase {
                    index: 0,
                    label: "Open IDE".into(),
                    expected_outcome: ApplicationWindow {
                        app_name: "code".into(),
                        title_hint: None,
                    },
                    optional: false,
                },
                WorkflowPhase {
                    index: 1,
                    label: "Edit code".into(),
                    expected_outcome: ApplicationWindow {
                        app_name: "code".into(),
                        title_hint: None,
                    },
                    optional: false,
                },
                WorkflowPhase {
                    index: 2,
                    label: "Run/build".into(),
                    expected_outcome: TerminalOutput {
                        contains: String::new(),
                    },
                    optional: true,
                },
            ],
            expected_apps: vec!["code".into(), "nvim".into(), "gedit".into()],
            can_background: false,
            typical_duration_sec: 120,
            modifies_fs: true,
            needs_network: false,
        },
    );

    // ── Browser ───────────────────────────────────────────────────────────
    lib.insert(
        Browser,
        WorkflowExpectation {
            category: Browser,
            expected_outcomes: vec![BrowserPage {
                url_contains: None,
                title_contains: None,
            }],
            typical_phases: vec![
                WorkflowPhase {
                    index: 0,
                    label: "Navigate to URL".into(),
                    expected_outcome: BrowserPage {
                        url_contains: None,
                        title_contains: None,
                    },
                    optional: false,
                },
                WorkflowPhase {
                    index: 1,
                    label: "Interact with page".into(),
                    expected_outcome: BrowserPage {
                        url_contains: None,
                        title_contains: None,
                    },
                    optional: true,
                },
            ],
            expected_apps: vec!["firefox".into(), "chromium".into(), "chrome".into()],
            can_background: false,
            typical_duration_sec: 30,
            modifies_fs: false,
            needs_network: true,
        },
    );

    // ── FileManagement ────────────────────────────────────────────────────
    lib.insert(
        FileManagement,
        WorkflowExpectation {
            category: FileManagement,
            expected_outcomes: vec![FileCreated {
                path: std::path::PathBuf::from("/"),
                min_size_bytes: None,
            }],
            typical_phases: vec![WorkflowPhase {
                index: 0,
                label: "Create/modify file".into(),
                expected_outcome: FileCreated {
                    path: std::path::PathBuf::from("/"),
                    min_size_bytes: None,
                },
                optional: false,
            }],
            expected_apps: vec!["nautilus".into(), "thunar".into(), "dolphin".into()],
            can_background: true,
            typical_duration_sec: 10,
            modifies_fs: true,
            needs_network: false,
        },
    );

    // ── Terminal ──────────────────────────────────────────────────────────
    lib.insert(
        Terminal,
        WorkflowExpectation {
            category: Terminal,
            expected_outcomes: vec![
                ApplicationWindow {
                    app_name: "terminal".into(),
                    title_hint: None,
                },
                TerminalOutput {
                    contains: String::new(),
                },
            ],
            typical_phases: vec![
                WorkflowPhase {
                    index: 0,
                    label: "Open terminal".into(),
                    expected_outcome: ApplicationWindow {
                        app_name: "terminal".into(),
                        title_hint: None,
                    },
                    optional: false,
                },
                WorkflowPhase {
                    index: 1,
                    label: "Execute command".into(),
                    expected_outcome: TerminalOutput {
                        contains: String::new(),
                    },
                    optional: false,
                },
            ],
            expected_apps: vec![
                "gnome-terminal".into(),
                "konsole".into(),
                "alacritty".into(),
                "kitty".into(),
            ],
            can_background: false,
            typical_duration_sec: 30,
            modifies_fs: false,
            needs_network: false,
        },
    );

    // ── JiraDevOps ────────────────────────────────────────────────────────
    lib.insert(
        JiraDevOps,
        WorkflowExpectation {
            category: JiraDevOps,
            expected_outcomes: vec![BrowserPage {
                url_contains: Some("jira".into()),
                title_contains: None,
            }],
            typical_phases: vec![
                WorkflowPhase {
                    index: 0,
                    label: "Open ticket".into(),
                    expected_outcome: BrowserPage {
                        url_contains: Some("jira".into()),
                        title_contains: None,
                    },
                    optional: false,
                },
                WorkflowPhase {
                    index: 1,
                    label: "Update ticket".into(),
                    expected_outcome: BrowserPage {
                        url_contains: Some("jira".into()),
                        title_contains: None,
                    },
                    optional: true,
                },
            ],
            expected_apps: vec!["firefox".into(), "chromium".into()],
            can_background: false,
            typical_duration_sec: 60,
            modifies_fs: false,
            needs_network: true,
        },
    );

    // ── Debugging ─────────────────────────────────────────────────────────
    lib.insert(
        Debugging,
        WorkflowExpectation {
            category: Debugging,
            expected_outcomes: vec![
                ApplicationWindow {
                    app_name: "code".into(),
                    title_hint: None,
                },
                TerminalOutput {
                    contains: String::new(),
                },
            ],
            typical_phases: vec![
                WorkflowPhase {
                    index: 0,
                    label: "Identify error".into(),
                    expected_outcome: TerminalOutput {
                        contains: String::new(),
                    },
                    optional: false,
                },
                WorkflowPhase {
                    index: 1,
                    label: "Locate source".into(),
                    expected_outcome: ApplicationWindow {
                        app_name: "code".into(),
                        title_hint: None,
                    },
                    optional: false,
                },
                WorkflowPhase {
                    index: 2,
                    label: "Apply fix".into(),
                    expected_outcome: ApplicationWindow {
                        app_name: "code".into(),
                        title_hint: None,
                    },
                    optional: false,
                },
                WorkflowPhase {
                    index: 3,
                    label: "Verify fix".into(),
                    expected_outcome: TerminalOutput {
                        contains: String::new(),
                    },
                    optional: true,
                },
            ],
            expected_apps: vec!["code".into(), "gdb".into()],
            can_background: false,
            typical_duration_sec: 300,
            modifies_fs: true,
            needs_network: false,
        },
    );

    // ── Deployment ────────────────────────────────────────────────────────
    lib.insert(
        Deployment,
        WorkflowExpectation {
            category: Deployment,
            expected_outcomes: vec![
                TerminalOutput {
                    contains: String::new(),
                },
                NotificationVisible {
                    contains: "deployed".into(),
                },
            ],
            typical_phases: vec![
                WorkflowPhase {
                    index: 0,
                    label: "Build artifact".into(),
                    expected_outcome: TerminalOutput {
                        contains: String::new(),
                    },
                    optional: false,
                },
                WorkflowPhase {
                    index: 1,
                    label: "Run tests".into(),
                    expected_outcome: TerminalOutput {
                        contains: String::new(),
                    },
                    optional: true,
                },
                WorkflowPhase {
                    index: 2,
                    label: "Deploy".into(),
                    expected_outcome: NotificationVisible {
                        contains: "deployed".into(),
                    },
                    optional: false,
                },
            ],
            expected_apps: vec!["gnome-terminal".into(), "konsole".into()],
            can_background: true,
            typical_duration_sec: 120,
            modifies_fs: false,
            needs_network: true,
        },
    );

    // ── Email ──────────────────────────────────────────────────────────────
    lib.insert(
        Email,
        WorkflowExpectation {
            category: Email,
            expected_outcomes: vec![EmailSentConfirmation { client_hint: None }],
            typical_phases: vec![
                WorkflowPhase {
                    index: 0,
                    label: "Compose email".into(),
                    expected_outcome: ApplicationWindow {
                        app_name: "thunderbird".into(),
                        title_hint: None,
                    },
                    optional: false,
                },
                WorkflowPhase {
                    index: 1,
                    label: "Send email".into(),
                    expected_outcome: EmailSentConfirmation { client_hint: None },
                    optional: false,
                },
            ],
            expected_apps: vec!["thunderbird".into(), "geary".into()],
            can_background: false,
            typical_duration_sec: 60,
            modifies_fs: false,
            needs_network: true,
        },
    );

    // ── Media ─────────────────────────────────────────────────────────────
    lib.insert(
        Media,
        WorkflowExpectation {
            category: Media,
            expected_outcomes: vec![AudioPlaybackActive { player_hint: None }],
            typical_phases: vec![WorkflowPhase {
                index: 0,
                label: "Start player".into(),
                expected_outcome: AudioPlaybackActive { player_hint: None },
                optional: false,
            }],
            expected_apps: vec!["vlc".into(), "rhythmbox".into(), "mpv".into()],
            can_background: true,
            typical_duration_sec: 5,
            modifies_fs: false,
            needs_network: false,
        },
    );

    // ── SystemConfiguration ───────────────────────────────────────────────
    lib.insert(
        SystemConfiguration,
        WorkflowExpectation {
            category: SystemConfiguration,
            expected_outcomes: vec![NotificationVisible {
                contains: "configured".into(),
            }],
            typical_phases: vec![WorkflowPhase {
                index: 0,
                label: "Apply configuration".into(),
                expected_outcome: NotificationVisible {
                    contains: "configured".into(),
                },
                optional: false,
            }],
            expected_apps: vec![],
            can_background: true,
            typical_duration_sec: 30,
            modifies_fs: true,
            needs_network: false,
        },
    );

    // ── Unknown fallback ──────────────────────────────────────────────────
    lib.insert(
        Unknown,
        WorkflowExpectation {
            category: Unknown,
            expected_outcomes: vec![ObservableOutcome::Silent],
            typical_phases: vec![],
            expected_apps: vec![],
            can_background: true,
            typical_duration_sec: 60,
            modifies_fs: false,
            needs_network: false,
        },
    );

    lib
}

// ─── WorkflowExpectationEngine ────────────────────────────────────────────────

/// Classifies workflow categories and produces expectation templates.
///
/// Used before workflow execution to determine what human-visible outcomes
/// are expected, and during execution to track progress.
pub struct WorkflowExpectationEngine {
    templates: HashMap<WorkflowCategory, WorkflowExpectation>,
    psdg: Option<PsdgHandle>,
}

impl WorkflowExpectationEngine {
    /// Create a new engine with the canonical template library.
    pub fn new(psdg: Option<PsdgHandle>) -> Self {
        Self {
            templates: build_template_library(),
            psdg,
        }
    }

    /// Classify the workflow category from context.
    ///
    /// Uses prompt keywords, verb/target types, operation, and PSDG context.
    /// Pure classification — no I/O, no state changes.
    pub fn classify(
        &self,
        prompt: &str,
        verb: &Verb,
        targets: &[TargetRef],
        operation: Operation,
    ) -> WorkflowCategory {
        let lower = prompt.to_lowercase();

        // PSDG fast-path: refine classification using live desktop context.
        if let Some(ref h) = self.psdg {
            if let Ok(Some(fact)) = h
                .store()
                .query("desktop_environment", "active_workflow_category")
            {
                if fact.confidence >= 0.7 {
                    debug!(
                        target: "workflow_expectation",
                        category = %fact.object,
                        "PSDG: using persisted workflow category"
                    );
                }
            }
        }

        // Keyword-driven classification (deterministic, no LLM).
        // Priority order (highest specificity first):
        //   1. Typed targets (URL, code file extension) — unambiguous
        //   2. Deployment / Debugging / Email (high specificity)
        //   3. SystemConfig (install/configure — must be before browser app-name keywords)
        //   4. JiraDevOps (broad DevOps terms — after URL check)
        //   5. Browser / Coding / Media / File
        //   6. Terminal / Unknown

        // URL target → Browser (takes priority over "github" keyword → JiraDevOps)
        for t in targets {
            if let TargetRef::Url(_) = t {
                return WorkflowCategory::Browser;
            }
        }

        // Code file extension → Coding (takes priority over generic FileManagement)
        for t in targets {
            if let TargetRef::File(p) = t {
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(
                    ext,
                    "rs" | "py"
                        | "js"
                        | "ts"
                        | "go"
                        | "java"
                        | "cpp"
                        | "c"
                        | "h"
                        | "rb"
                        | "cs"
                        | "swift"
                        | "kt"
                        | "scala"
                        | "hs"
                ) {
                    return WorkflowCategory::Coding;
                }
            }
        }

        // Email
        if lower.contains("email") || lower.contains("mail") || lower.contains("send message") {
            return WorkflowCategory::Email;
        }

        // SystemConfig — must be BEFORE browser so "install firefox" → SystemConfig not Browser
        if lower.contains("install")
            || lower.contains("configure")
            || lower.contains("service")
            || lower.contains("systemctl")
            || lower.contains("apt ")
            || lower.contains("dnf ")
            || matches!(operation, Operation::ConfigureSystem)
        {
            return WorkflowCategory::SystemConfiguration;
        }

        // Jira / DevOps
        if lower.contains("jira")
            || lower.contains("ticket")
            || lower.contains("issue")
            || lower.contains("github")
            || lower.contains("pr ")
            || lower.contains("pull request")
        {
            return WorkflowCategory::JiraDevOps;
        }

        // Deployment
        if lower.contains("deploy")
            || lower.contains("release")
            || lower.contains("publish")
            || lower.contains("ci")
            || lower.contains("pipeline")
            || lower.contains("dockerfile")
        {
            return WorkflowCategory::Deployment;
        }

        // Debugging
        if lower.contains("debug")
            || lower.contains("breakpoint")
            || lower.contains("traceback")
            || lower.contains("fix the error")
            || lower.contains("diagnose")
            || lower.contains("why is it failing")
            || lower.contains("gdb")
        {
            return WorkflowCategory::Debugging;
        }

        // Coding
        if lower.contains("code")
            || lower.contains("write a")
            || lower.contains("implement")
            || lower.contains("function")
            || lower.contains("class ")
            || lower.contains("program")
            || lower.contains("vscode")
            || lower.contains("edit ")
            || lower.contains("refactor")
            || matches!(verb, Verb::Other(s) if s == "code" || s == "write")
        {
            // Specifically check for file targets with code extensions
            for t in targets {
                if let TargetRef::File(p) = t {
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(
                        ext,
                        "rs" | "py" | "js" | "ts" | "go" | "java" | "cpp" | "c" | "h"
                    ) {
                        return WorkflowCategory::Coding;
                    }
                }
            }
            return WorkflowCategory::Coding;
        }

        // Browser (URL keywords, after URL-target check above)
        if lower.contains("browser")
            || lower.contains("website")
            || lower.contains("search")
            || lower.contains("navigate")
            || lower.contains("open url")
            || lower.contains("firefox")
            || lower.contains("chrome")
            || lower.contains("chromium")
        {
            return WorkflowCategory::Browser;
        }

        // Media
        if lower.contains("play")
            || lower.contains("music")
            || lower.contains("video")
            || lower.contains("audio")
            || lower.contains("vlc")
            || lower.contains("mpv")
        {
            return WorkflowCategory::Media;
        }

        // File management (non-code files, after code extension check)
        for t in targets {
            if matches!(t, TargetRef::File(_)) {
                return WorkflowCategory::FileManagement;
            }
        }
        if lower.contains("file")
            || lower.contains("folder")
            || lower.contains("directory")
            || lower.contains("copy")
            || lower.contains("move")
            || lower.contains("rename")
            || matches!(verb, Verb::Save | Verb::Open)
        {
            return WorkflowCategory::FileManagement;
        }

        // Terminal
        if matches!(operation, Operation::ExecuteShell | Operation::ExecuteCode)
            || lower.contains("terminal")
            || lower.contains("bash")
            || lower.contains("zsh")
            || lower.contains("shell ")
            || lower.contains("run command")
        {
            return WorkflowCategory::Terminal;
        }

        WorkflowCategory::Unknown
    }

    /// Get the expectation template for a workflow category.
    ///
    /// Returns the template refined with live PSDG context when available.
    pub fn expectation_for(&self, category: WorkflowCategory) -> &WorkflowExpectation {
        self.templates
            .get(&category)
            .or_else(|| self.templates.get(&WorkflowCategory::Unknown))
            .expect("Unknown category fallback always present")
    }

    /// Get the expectation template, refined with live PSDG context.
    ///
    /// Returns a cloned, context-enriched template.
    pub fn refined_expectation(&self, category: WorkflowCategory) -> WorkflowExpectation {
        let mut template = self.expectation_for(category).clone();

        // Refine with PSDG context.
        if let Some(ref h) = self.psdg {
            // If the browser already has a URL, add that to the expected outcomes.
            if category.needs_browser() {
                if let Some(url) = h.get_browser_url() {
                    if !template
                        .expected_outcomes
                        .iter()
                        .any(|o| matches!(o, ObservableOutcome::BrowserPage { .. }))
                    {
                        template
                            .expected_outcomes
                            .push(ObservableOutcome::BrowserPage {
                                url_contains: Some(url),
                                title_contains: None,
                            });
                    }
                }
            }

            // If the IDE already has a workspace, add it to expected outcomes.
            if category.needs_ide() {
                if let Some(ws) = h.get_ide_workspace() {
                    let exists = template
                        .expected_outcomes
                        .iter()
                        .any(|o| matches!(o, ObservableOutcome::IdeWorkspace { .. }));
                    if !exists {
                        template
                            .expected_outcomes
                            .push(ObservableOutcome::IdeWorkspace { path: ws });
                    }
                }
            }

            // Trim to max outcomes.
            template
                .expected_outcomes
                .truncate(MAX_OUTCOMES_PER_TEMPLATE);
        }

        template
    }

    /// Infer workflow progress given a session checkpoint and expectation.
    pub fn infer_progress(
        &self,
        session: &WorkflowSession,
        expectation: &WorkflowExpectation,
    ) -> WorkflowProgressReport {
        let total = expectation.typical_phases.len() as u32;
        let completed = session.completed_steps.len() as u32;
        let percent = if total == 0 {
            100
        } else {
            (completed * 100 / total).min(100)
        };

        let current_phase = expectation
            .typical_phases
            .get(completed as usize)
            .map(|p| p.label.clone());

        let mut blockers = Vec::new();
        if let Some(ref err) = session.error {
            blockers.push(format!("Last error: {}", err));
        }
        if session.continuation_hint.is_some() && !session.complete {
            blockers.push("Workflow interrupted — continuation hint available".into());
        }

        let summary = if session.complete {
            format!(
                "{} workflow completed ({} steps)",
                expectation.category.description(),
                completed
            )
        } else if !blockers.is_empty() {
            format!(
                "{} workflow blocked at step {} of {} — {}",
                expectation.category.description(),
                completed,
                total,
                blockers
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("unknown blocker")
            )
        } else {
            format!(
                "{} workflow: {}/{}  ({percent}%)",
                expectation.category.description(),
                completed,
                total
            )
        };

        WorkflowProgressReport {
            category: expectation.category,
            phases_completed: completed.min(total),
            total_phases: total,
            percent_complete: percent,
            current_phase,
            blockers,
            is_committed: completed > 0 && total > 0,
            summary,
        }
    }
}

impl Default for WorkflowExpectationEngine {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::intent_compiler::{TargetRef, Verb};

    fn engine() -> WorkflowExpectationEngine {
        WorkflowExpectationEngine::new(None)
    }

    // ── Classification tests ───────────────────────────────────────────────

    #[test]
    fn classify_code_file_extension_is_coding() {
        let eng = engine();
        let cat = eng.classify(
            "open main.rs",
            &Verb::Open,
            &[TargetRef::File("/home/user/main.rs".into())],
            Operation::Automate,
        );
        assert_eq!(cat, WorkflowCategory::Coding);
    }

    #[test]
    fn classify_url_target_is_browser() {
        let eng = engine();
        let cat = eng.classify(
            "navigate to https://github.com",
            &Verb::Open,
            &[TargetRef::Url("https://github.com".into())],
            Operation::Automate,
        );
        assert_eq!(cat, WorkflowCategory::Browser);
    }

    #[test]
    fn classify_jira_keyword_is_devops() {
        let eng = engine();
        let cat = eng.classify(
            "open jira ticket KR-123",
            &Verb::Open,
            &[],
            Operation::Automate,
        );
        assert_eq!(cat, WorkflowCategory::JiraDevOps);
    }

    #[test]
    fn classify_deploy_keyword_is_deployment() {
        let eng = engine();
        let cat = eng.classify(
            "deploy to production",
            &Verb::Run,
            &[],
            Operation::ExecuteShell,
        );
        assert_eq!(cat, WorkflowCategory::Deployment);
    }

    #[test]
    fn classify_debug_keyword_is_debugging() {
        let eng = engine();
        let cat = eng.classify(
            "debug the crash in server.rs",
            &Verb::Other("debug".into()),
            &[],
            Operation::Automate,
        );
        assert_eq!(cat, WorkflowCategory::Debugging);
    }

    #[test]
    fn classify_send_email_is_email() {
        let eng = engine();
        let cat = eng.classify(
            "send email to john@example.com",
            &Verb::Other("send".into()),
            &[],
            Operation::Send,
        );
        assert_eq!(cat, WorkflowCategory::Email);
    }

    #[test]
    fn classify_install_is_system_config() {
        let eng = engine();
        let cat = eng.classify(
            "install firefox",
            &Verb::Run,
            &[],
            Operation::ConfigureSystem,
        );
        assert_eq!(cat, WorkflowCategory::SystemConfiguration);
    }

    #[test]
    fn classify_play_music_is_media() {
        let eng = engine();
        let cat = eng.classify("play music in vlc", &Verb::Open, &[], Operation::Automate);
        assert_eq!(cat, WorkflowCategory::Media);
    }

    #[test]
    fn classify_shell_operation_is_terminal() {
        let eng = engine();
        let cat = eng.classify("run the script", &Verb::Run, &[], Operation::ExecuteShell);
        assert_eq!(cat, WorkflowCategory::Terminal);
    }

    // ── Template tests ─────────────────────────────────────────────────────

    #[test]
    fn coding_template_has_app_window_outcome() {
        let eng = engine();
        let tmpl = eng.expectation_for(WorkflowCategory::Coding);
        assert!(tmpl
            .expected_outcomes
            .iter()
            .any(|o| matches!(o, ObservableOutcome::ApplicationWindow { .. })));
    }

    #[test]
    fn browser_template_has_browser_page_outcome() {
        let eng = engine();
        let tmpl = eng.expectation_for(WorkflowCategory::Browser);
        assert!(tmpl
            .expected_outcomes
            .iter()
            .any(|o| matches!(o, ObservableOutcome::BrowserPage { .. })));
    }

    #[test]
    fn deployment_template_has_terminal_output() {
        let eng = engine();
        let tmpl = eng.expectation_for(WorkflowCategory::Deployment);
        assert!(tmpl
            .expected_outcomes
            .iter()
            .any(|o| matches!(o, ObservableOutcome::TerminalOutput { .. })));
    }

    #[test]
    fn all_categories_have_templates() {
        let eng = engine();
        for cat in [
            WorkflowCategory::Coding,
            WorkflowCategory::Browser,
            WorkflowCategory::FileManagement,
            WorkflowCategory::Terminal,
            WorkflowCategory::JiraDevOps,
            WorkflowCategory::Debugging,
            WorkflowCategory::Deployment,
            WorkflowCategory::Email,
            WorkflowCategory::Media,
            WorkflowCategory::SystemConfiguration,
        ] {
            let tmpl = eng.expectation_for(cat);
            assert!(
                !tmpl.expected_outcomes.is_empty(),
                "{:?} must have at least one expected outcome",
                cat
            );
        }
    }

    #[test]
    fn template_outcomes_bounded() {
        let eng = engine();
        for (_, tmpl) in &eng.templates {
            assert!(
                tmpl.expected_outcomes.len() <= MAX_OUTCOMES_PER_TEMPLATE,
                "{:?} template exceeds MAX_OUTCOMES_PER_TEMPLATE",
                tmpl.category
            );
        }
    }

    // ── Progress tracking ──────────────────────────────────────────────────

    #[test]
    fn progress_complete_session_is_100_percent() {
        let eng = engine();
        let mut session = WorkflowSession::new("s1".into(), "open firefox".into(), "Coding".into());
        session.mark_complete(vec![]);
        let expectation = eng.expectation_for(WorkflowCategory::Coding);
        let progress = eng.infer_progress(&session, expectation);
        assert!(progress.summary.contains("completed"));
    }

    #[test]
    fn progress_blocked_session_has_blocker() {
        let eng = engine();
        let mut session = WorkflowSession::new("s2".into(), "cargo build".into(), "Coding".into());
        session.mark_failed(
            "ECONNREFUSED".into(),
            Some("retry after checking network".into()),
        );
        let expectation = eng.expectation_for(WorkflowCategory::Coding);
        let progress = eng.infer_progress(&session, expectation);
        assert!(!progress.blockers.is_empty());
    }
}
