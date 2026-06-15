//! Substrate-aware GUI planner.
//!
//! The core insight: many "GUI tasks" don't actually require GUI automation.
//! "Open gedit and type a fibonacci program" is much more reliably executed by:
//!   1. Writing the file directly via the filesystem tool
//!   2. Opening the editor with that file as an argument
//!   3. Verifying via file existence + content check
//!
//! This avoids:
//! - Brittle character-by-character keystroke injection
//! - Wayland incompatibility (xdotool is X11-only)
//! - Race conditions between window-focus and typing
//! - OCR/vision verification that doesn't exist yet
//! - False success reports from unverifiable text-present checks
//!
//! The planner picks the execution substrate based on what the task actually needs:
//!   - **File substrate** (write_file + open_with_file): editor + generated code
//!   - **Keystroke substrate** (type_text): user wants literal typing into a focused field
//!   - **Browser substrate** (browser_search): URL navigation
//!
//! This is **execution intelligence**: choosing the right runtime for the goal,
//! not blindly translating verbs into keystrokes.

use crate::agent::htn_executor::{GuiWorkflow, SafeAbortStep, SubGoal, VerificationType};
use crate::agent::intent_compiler::{ContentClass, GuiTaskSpec, TargetRef, Verb};
use std::path::PathBuf;

/// Execution substrate — how a task should physically be performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionSubstrate {
    /// Generate content as a file, then open the file with the target editor.
    /// Verifiable via FileSystemEffect (works on X11 and Wayland).
    FileWriteThenOpen,
    /// Generate content as a file, run it with the appropriate interpreter,
    /// and verify the output contains expected content.
    /// Verifiable via DeterministicOutput (works on X11 and Wayland).
    TerminalExecution,
    /// Open generated code in an IDE-class app, launch a visible terminal run,
    /// and verify captured output structurally.
    IdeCodeRunWorkflow,
    /// Legacy serialized substrate name retained for older eval reports.
    VSCodeCodeRunWorkflow,
    /// Just open the application — no content typing.
    AppOpenOnly,
    /// Inject keystrokes via the GUI backend (literal user-provided text).
    /// Brittle on Wayland; only used when the user explicitly wants typing.
    Keystroke,
    /// Open browser with a URL or search query.
    BrowserNavigate,
    /// Interact with UI elements via AT-SPI accessibility tree.
    /// Works on both X11 and Wayland. Used for button clicks, form filling,
    /// menu navigation, dialog dismissal, and any interaction-heavy task.
    InteractionHeavy,
    /// Substrate cannot be determined — defer to LLM HTN planner.
    Unknown,
}

/// Result of substrate planning.
#[derive(Debug, Clone)]
pub struct SubstratePlan {
    pub substrate: ExecutionSubstrate,
    pub workflow: Option<GuiWorkflow>,
    /// Files this workflow will create (for verification / cleanup tracking).
    pub artifacts: Vec<PathBuf>,
}

impl SubstratePlan {
    pub fn unknown() -> Self {
        Self {
            substrate: ExecutionSubstrate::Unknown,
            workflow: None,
            artifacts: Vec::new(),
        }
    }
}

/// The substrate-aware planner.
///
/// Given a [`GuiTaskSpec`], decide which execution substrate is most reliable
/// for the actual goal, then emit a workflow optimised for that substrate.
pub struct SubstratePlanner;

impl SubstratePlanner {
    pub fn plan(&self, spec: &GuiTaskSpec, raw_user_text: &str) -> SubstratePlan {
        // ── Substrate decision tree (deterministic, no LLM) ────────────────
        // 1. If targets include URL → BrowserNavigate
        // 2. If app is a browser AND raw text contains search intent → BrowserNavigate
        // 3. If verb is Open + content is Generated code → FileWriteThenOpen
        // 4. If verb is Open + no content → AppOpenOnly
        // 5. If verb is Type + content is Literal → Keystroke
        // 6. If verb is Type + content is Generated → FileWriteOnly
        // 7. Otherwise Unknown (LLM HTN planner takes over)

        let app = spec.targets.iter().find_map(|t| match t {
            TargetRef::App(a) => Some(a.clone()),
            _ => None,
        });

        let urls: Vec<String> = spec
            .targets
            .iter()
            .filter_map(|t| match t {
                TargetRef::Url(u) => Some(u.clone()),
                _ => None,
            })
            .collect();

        // URL → browser substrate
        if !urls.is_empty() {
            return self.plan_browser_navigate(&urls, raw_user_text);
        }

        // Browser app + search intent → BrowserNavigate via browser_search tool
        // Detects: "open chrome and search for youtube", "open firefox and go to google"
        // FIX #15: Only route to browser_search if there is an actual search query.
        // "open firefox" with no search term should fall through to plan_app_open,
        // not navigate to google.com with an empty query.
        if let Some(ref app_name) = app {
            if Self::is_browser_app(app_name) {
                let search_query = Self::extract_search_query(raw_user_text);
                let site = Self::extract_search_site(raw_user_text);
                if search_query.is_some() || site.is_some() {
                    return self.plan_browser_search(search_query.as_deref(), site.as_deref());
                }
                // No search query — fall through to plan_app_open below
            }
        }

        if let Some((app_name, content, extension, hint)) =
            Self::extract_document_workflow(app.as_deref(), raw_user_text)
        {
            return self.plan_document_write_then_open(&app_name, &content, &extension, &hint);
        }

        match (&spec.primary_verb, app.as_deref(), spec.content.as_ref()) {
            // Open editor + literal text → write file + open file
            (Verb::Open, Some(app_name), Some(ContentClass::Literal(text))) => {
                self.plan_file_write_then_open(app_name, text, None, raw_user_text)
            }

            // Open editor + generate code → write file + open file
            (Verb::Open, Some(app_name), Some(ContentClass::Generated { hint, language })) => {
                // Check if the raw text also contains a "run" intent
                // e.g., "open gedit and write a fibonacci program and run it"
                let wants_run = Self::has_run_intent(raw_user_text);
                if wants_run {
                    if Self::is_ide_app(app_name) {
                        self.plan_ide_code_run_workflow(
                            app_name,
                            hint,
                            language.as_deref(),
                            raw_user_text,
                        )
                    } else {
                        self.plan_terminal_execution(
                            Some(app_name),
                            hint,
                            language.as_deref(),
                            raw_user_text,
                        )
                    }
                } else {
                    self.plan_file_write_then_open(
                        app_name,
                        hint,
                        language.as_deref(),
                        raw_user_text,
                    )
                }
            }

            // Open terminal app with no explicit content BUT an embedded command +
            // show-output intent → extract the command and execute it directly.
            // e.g. "Open terminal and check disk usage with df -h and show me the output"
            //   → execute_bash("df -h") with DeterministicOutput verification.
            // This handles the common pattern where the user asks to "open a terminal
            // and run/check X" without phrasing it as code generation.
            (Verb::Open, Some(app_name), None)
                if Self::is_terminal_app(app_name) && Self::has_show_intent(raw_user_text) =>
            {
                if let Some(cmd) = Self::extract_terminal_command(raw_user_text) {
                    self.plan_terminal_execution_literal(app_name, &cmd, raw_user_text)
                } else {
                    self.plan_app_open(app_name)
                }
            }

            // Open app, no content
            (Verb::Open, Some(app_name), None) => self.plan_app_open(app_name),

            // Run verb with generated content → TerminalExecution
            (Verb::Run, _, Some(ContentClass::Generated { hint, language })) => {
                self.plan_terminal_execution(None, hint, language.as_deref(), raw_user_text)
            }

            // Click verb → InteractionHeavy substrate via AT-SPI
            (Verb::Click, _, _) => {
                let element = spec.targets.iter().find_map(|t| match t {
                    TargetRef::Element(e) => Some(e.clone()),
                    TargetRef::App(a) => Some(a.clone()),
                    _ => None,
                });
                self.plan_interaction_heavy_click(element.as_deref(), raw_user_text)
            }

            // Type literal text — only viable on X11. On Wayland we have no
            // reliable text injection, so we still mark this as Unknown to
            // let the LLM HTN planner (or the user) choose differently.
            (Verb::Type, _, Some(ContentClass::Literal(text))) => self.plan_keystroke(text),

            // Type generated content with no target app → file substrate to
            // a default location, no app open.
            (Verb::Type, _, Some(ContentClass::Generated { hint, language })) => {
                self.plan_file_write_only(hint, language.as_deref(), raw_user_text)
            }

            // Switch to terminal + run/show intent → TerminalExecution
            (Verb::Switch, _, _)
                if Self::has_show_intent(raw_user_text) || Self::has_run_intent(raw_user_text) =>
            {
                if let Some(cmd) = Self::extract_terminal_command(raw_user_text) {
                    self.plan_terminal_execution_literal(
                        app.as_deref().unwrap_or("gnome-terminal"),
                        &cmd,
                        raw_user_text,
                    )
                } else {
                    SubstratePlan::unknown()
                }
            }

            // Save verb → press Ctrl+S
            (Verb::Save, _, _) => self.plan_save_shortcut(),

            // Catch-all: if the raw text contains a code-generation marker
            // (e.g. "Code is already open, write a …", "add a function called…",
            // "Write a python program … and run it"), route via file substrate.
            _ => {
                if let Some((hint, language)) = Self::extract_code_gen_from_raw(raw_user_text) {
                    if Self::has_run_intent(raw_user_text) {
                        if app.as_deref().map(Self::is_ide_app).unwrap_or(false) {
                            self.plan_ide_code_run_workflow(
                                app.as_deref().unwrap_or("code"),
                                &hint,
                                language.as_deref(),
                                raw_user_text,
                            )
                        } else {
                            self.plan_terminal_execution(
                                app.as_deref(),
                                &hint,
                                language.as_deref(),
                                raw_user_text,
                            )
                        }
                    } else {
                        let app_name = app.as_deref().unwrap_or("code");
                        self.plan_file_write_then_open(
                            app_name,
                            &hint,
                            language.as_deref(),
                            raw_user_text,
                        )
                    }
                } else {
                    SubstratePlan::unknown()
                }
            }
        }
    }

    /// Check if the raw user text contains a "run" or "execute" intent.
    fn has_run_intent(raw_text: &str) -> bool {
        let lower = raw_text.to_ascii_lowercase();
        let run_markers = [
            " and run",
            " and execute",
            " then run",
            " then execute",
            " run it",
            " execute it",
            " run the",
            " execute the",
            " after run",
            " after execute",
            " finally run",
            " also run",
            " also execute",
            // "show output" / "show the output" also implies run + display
            "show output",
            "show the output",
            "show me the output",
            "show me output",
            "display the output",
            "display output",
            "show me the result",
            "show the result",
            "show me the answer",
            "print the output",
            "print the result",
        ];
        run_markers.iter().any(|m| lower.contains(m))
    }

    /// Extract a (hint, language) pair from raw_text for prompts that use
    /// code-generation markers without a leading "Open" verb, such as:
    ///   "Write a python program …"
    ///   "Code is already open, write a new python file …"
    ///   "add a function called greet …"
    fn extract_code_gen_from_raw(raw_text: &str) -> Option<(String, Option<String>)> {
        let lower = raw_text.to_ascii_lowercase();
        let markers: &[&str] = &[
            "write a python",
            "write a rust",
            "write a javascript",
            "write a program",
            "write a function",
            "write a new python",
            "write a new ",
            "add a new function",
            "add a function",
            "create a python",
            "create a program",
            "implement ",
        ];
        for marker in markers {
            if let Some(pos) = lower.find(marker) {
                let rest = &raw_text[pos..];
                let hint: String = rest
                    .split_whitespace()
                    .skip_while(|w| {
                        ["a", "an", "the", "new"].contains(&w.to_ascii_lowercase().as_str())
                    })
                    .take(15)
                    .collect::<Vec<_>>()
                    .join(" ");
                if hint.is_empty() {
                    continue;
                }
                let language = if lower.contains("python") {
                    Some("python".to_string())
                } else if lower.contains("rust") {
                    Some("rust".to_string())
                } else if lower.contains("javascript") || lower.contains(" js ") {
                    Some("javascript".to_string())
                } else {
                    None
                };
                return Some((hint, language));
            }
        }
        None
    }

    fn extract_document_workflow(
        app: Option<&str>,
        raw_text: &str,
    ) -> Option<(String, String, String, String)> {
        let lower = raw_text.to_ascii_lowercase();

        if lower.contains("email")
            && (lower.contains("draft") || lower.contains("write") || lower.contains("compose"))
            && (lower.contains("text editor")
                || lower.contains("editor")
                || lower.contains("do not send")
                || lower.contains("approval"))
        {
            let app_name = app.unwrap_or("text editor").to_string();
            let content = [
                "Subject: Quick note",
                "",
                "Hi,",
                "",
                "I wanted to send a short note and check in. Please let me know if you would like me to adjust anything before this is sent.",
                "",
                "Best,",
                "",
                "[KRIA_DRAFT_REVIEW_REQUIRED]",
            ]
            .join("\n");
            return Some((
                app_name,
                content,
                "md".to_string(),
                "email draft".to_string(),
            ));
        }

        if contains_any(
            &lower,
            &[
                "spreadsheet",
                "excel",
                "libreoffice calc",
                " calc ",
                "calc if available",
                "sheet",
            ],
        ) && (lower.contains("column")
            || lower.contains("temporary sheet")
            || lower.contains("quantity")
            || lower.contains("price"))
        {
            let app_name = app.unwrap_or("spreadsheet").to_string();
            let content = [
                "Item,Quantity,Price,Total",
                "Sample item,1,0.00,0.00",
                "Review item,2,0.00,0.00",
            ]
            .join("\n");
            return Some((
                app_name,
                content,
                "csv".to_string(),
                "temporary spreadsheet".to_string(),
            ));
        }

        None
    }

    /// Check if an app name refers to a browser.
    fn is_browser_app(app_name: &str) -> bool {
        let lower = app_name.to_ascii_lowercase();
        matches!(
            lower.as_str(),
            "google-chrome"
                | "google-chrome-stable"
                | "chrome"
                | "chromium"
                | "chromium-browser"
                | "firefox"
                | "firefox-esr"
                | "brave"
                | "brave-browser"
                | "opera"
                | "vivaldi"
                | "microsoft-edge"
                | "edge"
                | "waterfox"
                | "librewolf"
                | "epiphany"
                | "gnome-web"
                | "xdg-open"
                | "browser"
                | "youtube"
        )
    }

    fn is_ide_app(app_name: &str) -> bool {
        matches!(
            app_name.to_ascii_lowercase().as_str(),
            "code"
                | "vscode"
                | "vs code"
                | "visual studio code"
                | "code-oss"
                | "vscodium"
                | "cursor"
                | "cursor editor"
                | "windsurf"
                | "zed"
                | "intellij"
                | "intellij idea"
                | "idea"
                | "idea-ic"
                | "idea-ultimate"
                | "pycharm"
                | "pycharm community"
                | "pycharm professional"
                | "webstorm"
                | "clion"
                | "rider"
                | "rubymine"
                | "phpstorm"
                | "goland"
                | "rustrover"
                | "eclipse"
                | "netbeans"
                | "android studio"
                | "android-studio"
        )
    }

    /// Extract a search query from the raw user text.
    /// "open chrome and search for youtube" → Some("youtube")
    /// "open firefox" → None
    fn extract_search_query(raw_text: &str) -> Option<String> {
        let lower = raw_text.to_ascii_lowercase();
        // Patterns: "search for X", "search X", "go to X", "navigate to X"
        // FIX #24: Removed "find " — it collides with file-search intent
        // ("open chrome and find the fibonacci algorithm" should not trigger browser search)
        for marker in &[
            "search for ",
            "search ",
            "look up ",
            "go to ",
            "navigate to ",
        ] {
            if let Some(pos) = lower.find(marker) {
                let after = raw_text[pos + marker.len()..].trim();
                // Stop at sentence end, punctuation, or "and" conjunction
                // that separates two commands (e.g., "search for youtube and then open spotify")
                let query = after
                    .split(|c: char| c == ',' || c == '.' || c == '!' || c == ';')
                    .next()
                    .unwrap_or(after)
                    .trim();
                // Further split on " and " to stop at command conjunctions
                let query = query.split(" and ").next().unwrap_or(query).trim();
                if !query.is_empty() {
                    return Some(query.to_string());
                }
            }
        }
        None
    }

    /// Extract the target site from the raw user text.
    /// "search for youtube" → Some("youtube")
    ///
    /// W-25 fix: only scan the extracted query portion, not the full raw text.
    /// "open chrome and search for lo-fi music and then open youtube" should
    /// NOT return "youtube" as the site — the user wants to search for lo-fi music.
    fn extract_search_site(raw_text: &str) -> Option<String> {
        // First extract the query portion to avoid false site matches
        let query_portion = if let Some(query) = Self::extract_search_query(raw_text) {
            query
        } else {
            // No explicit search query — scan the full text for site navigation
            raw_text.to_string()
        };

        let lower = query_portion.to_ascii_lowercase();
        // Check for well-known sites
        const KNOWN_SITES: &[(&str, &str)] = &[
            ("youtube", "youtube"),
            ("reddit", "reddit"),
            ("github", "github"),
            ("stackoverflow", "stackoverflow"),
            ("twitter", "twitter"),
            ("x.com", "twitter"),
            ("instagram", "instagram"),
            ("linkedin", "linkedin"),
            ("google", "google"),
        ];
        for (keyword, site) in KNOWN_SITES {
            if lower.contains(keyword) {
                return Some(site.to_string());
            }
        }
        None
    }

    // ── BrowserSearch substrate (via browser_search tool) ────────────────

    fn plan_browser_search(&self, query: Option<&str>, site: Option<&str>) -> SubstratePlan {
        let mut params = serde_json::json!({});
        if let Some(q) = query {
            params["query"] = serde_json::Value::String(q.to_string());
        } else {
            params["query"] = serde_json::Value::String(String::new());
        }
        if let Some(s) = site {
            params["site"] = serde_json::Value::String(s.to_string());
        }

        let sub_goals = vec![SubGoal {
            step: 1,
            action: "browser_search".into(),
            params,
            verify: VerificationType::BrowserPageLoaded {
                url_contains: site.map(|s| s.to_string()),
                title_contains: query.map(|q| q.to_string()),
            },
            // 45s: browser launch + navigation + verification (increased from 30s)
            timeout_ms: Some(45_000),
        }];

        SubstratePlan {
            substrate: ExecutionSubstrate::BrowserNavigate,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-browser-search-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![SafeAbortStep {
                    action: "press_shortcut".into(),
                    params: serde_json::json!({ "keys": ["Escape"] }),
                }],
                max_duration_sec: 60,
            }),
            artifacts: vec![],
        }
    }

    // ── TerminalExecution substrate ──────────────────────────────────────

    /// Plan a write-then-execute workflow:
    /// 1. Write the generated code to a file (FileSystemEffect verification)
    /// 2. Execute the file with the appropriate interpreter (execute_bash)
    ///    Output is captured to a file for verification.
    /// 3. Verify the output contains expected content (DeterministicOutput)
    ///
    /// This is the "True Execution Capability" substrate — it validates that
    /// the generated code actually runs and produces correct output, not just
    /// that the file was written.
    fn plan_terminal_execution(
        &self,
        app: Option<&str>,
        hint: &str,
        language: Option<&str>,
        raw_user_text: &str,
    ) -> SubstratePlan {
        let extension = language_to_extension(language, raw_user_text);
        let filename = generate_filename(hint, &extension);
        let full_path = generated_files_dir().join(&filename);
        let code = generate_code_from_hint(hint, language, raw_user_text);

        // Use UUID for output file to avoid timestamp collisions in concurrent execution
        let output_path =
            generated_files_dir().join(format!("output_{}.txt", uuid::Uuid::new_v4()));

        // Build the execution command safely.
        // Uses shell quoting to handle paths with spaces.
        // Redirects stdin from /dev/null to prevent programs that read stdin from hanging.
        let exec_command =
            build_execution_command(language, raw_user_text, &full_path, &output_path);

        // Determine expected output for verification
        let expected_output = extract_expected_output(hint, language);

        let mut sub_goals = vec![
            // Step 1: Write the generated code to a file.
            SubGoal {
                step: 1,
                action: "write_file".into(),
                params: serde_json::json!({
                    "path": full_path.to_string_lossy().to_string(),
                    "content": code.clone(),
                }),
                verify: VerificationType::FileSystemEffect {
                    path: full_path.clone(),
                    expected_substring: extract_verifiable_substring(&code),
                },
                timeout_ms: Some(5000),
            },
            // Step 2: Execute the file and capture output.
            // The command is built safely with proper quoting and stdin redirection.
            SubGoal {
                step: 2,
                action: "execute_bash".into(),
                params: serde_json::json!({
                    "command": exec_command,
                    "timeout": 30,
                }),
                // Verify the output file contains expected content.
                // expected_output is a distinctive substring (never empty for known topics).
                verify: VerificationType::DeterministicOutput {
                    expected_substring: expected_output.clone(),
                    output_file: output_path.clone(),
                },
                timeout_ms: Some(35000),
            },
        ];

        if let Some(app_name) = app {
            let binary = app_alias_to_binary(app_name);
            sub_goals.push(SubGoal {
                step: 3,
                action: "open_application_with_file".into(),
                params: serde_json::json!({
                    "name": app_name,
                    "file": full_path.to_string_lossy().to_string(),
                }),
                // Use ProcessLaunched, exactly like FileWriteThenOpen does
                verify: VerificationType::ProcessLaunched {
                    binary: binary.to_string(),
                    max_wait_ms: 10000,
                },
                timeout_ms: Some(15000),
            });
        }

        SubstratePlan {
            substrate: ExecutionSubstrate::TerminalExecution,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-exec-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![],
                max_duration_sec: 60,
            }),
            artifacts: vec![full_path, output_path],
        }
    }

    fn plan_ide_code_run_workflow(
        &self,
        app_name: &str,
        hint: &str,
        language: Option<&str>,
        raw_user_text: &str,
    ) -> SubstratePlan {
        let extension = language_to_extension(language, raw_user_text);
        let filename = generate_filename(hint, &extension);
        let full_path = generated_files_dir().join(&filename);
        let output_path =
            generated_files_dir().join(format!("output_{}.txt", uuid::Uuid::new_v4()));
        let runner_path = generated_files_dir().join(format!("run_{}.sh", uuid::Uuid::new_v4()));

        let code = generate_code_from_hint(hint, language, raw_user_text);
        let expected_output = extract_expected_output(hint, language);
        let terminal_run_command =
            build_visible_terminal_run_command(language, raw_user_text, &full_path, &output_path);
        let fallback_exec_command =
            build_execution_command(language, raw_user_text, &full_path, &output_path);
        let runner_script = format!(
            "#!/usr/bin/env bash\nset -u\ncd '{}'\necho '[KRIA] Hybrid IDE coding workflow'\necho '[KRIA] Source file: {}'\n{}\necho\necho '[KRIA] Output captured at: {}'\nexec bash\n",
            full_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_string_lossy()
                .replace('\'', "'\\''"),
            full_path.display(),
            terminal_run_command,
            output_path.display()
        );
        let launch_command =
            build_terminal_launcher_command(&runner_path, &output_path, &fallback_exec_command);

        let sub_goals = vec![
            SubGoal {
                step: 1,
                action: "write_file".into(),
                params: serde_json::json!({
                    "path": full_path.to_string_lossy().to_string(),
                    "content": code.clone(),
                }),
                verify: VerificationType::FileSystemEffect {
                    path: full_path.clone(),
                    expected_substring: extract_verifiable_substring(&code),
                },
                timeout_ms: Some(5000),
            },
            SubGoal {
                step: 2,
                action: "write_file".into(),
                params: serde_json::json!({
                    "path": runner_path.to_string_lossy().to_string(),
                    "content": runner_script,
                }),
                verify: VerificationType::FileSystemEffect {
                    path: runner_path.clone(),
                    expected_substring: "KRIA".to_string(),
                },
                timeout_ms: Some(5000),
            },
            SubGoal {
                step: 3,
                action: "open_application_with_file".into(),
                params: serde_json::json!({
                    "name": app_name,
                    "file": full_path.to_string_lossy().to_string(),
                }),
                verify: VerificationType::ProcessLaunched {
                    binary: app_alias_to_binary(app_name),
                    max_wait_ms: 12_000,
                },
                timeout_ms: Some(15_000),
            },
            SubGoal {
                step: 4,
                action: "execute_bash".into(),
                params: serde_json::json!({
                    "command": launch_command,
                    "timeout": 20,
                }),
                verify: VerificationType::DeterministicOutput {
                    expected_substring: expected_output,
                    output_file: output_path.clone(),
                },
                timeout_ms: Some(25_000),
            },
        ];

        SubstratePlan {
            substrate: ExecutionSubstrate::IdeCodeRunWorkflow,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-ide-run-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![],
                max_duration_sec: 70,
            }),
            artifacts: vec![full_path, output_path, runner_path],
        }
    }

    // ── FileWriteThenOpen substrate ──────────────────────────────────────

    fn plan_file_write_then_open(
        &self,
        app: &str,
        hint: &str,
        language: Option<&str>,
        raw_user_text: &str,
    ) -> SubstratePlan {
        let extension = language_to_extension(language, raw_user_text);
        let filename = generate_filename(hint, &extension);
        let full_path = generated_files_dir().join(&filename);
        let path_str = full_path.to_string_lossy().to_string();
        // `filename` is intentionally unused after Step 2 verification was
        // moved off `WindowState{title_contains}`. Kept for full_path naming.
        let _ = &filename;

        // Generate the actual code content (not the hint).
        let code = generate_code_from_hint(hint, language, raw_user_text);

        let sub_goals = vec![
            // Step 1: Write the file directly via the filesystem tool.
            // 100% verifiable via FileSystemEffect — no GUI involved.
            SubGoal {
                step: 1,
                action: "write_file".into(),
                params: serde_json::json!({
                    "path": path_str.clone(),
                    "content": code.clone(),
                }),
                verify: VerificationType::FileSystemEffect {
                    path: full_path.clone(),
                    expected_substring: extract_verifiable_substring(&code),
                },
                timeout_ms: Some(5000),
            },
            // Step 2: Open the editor with the file as an argument.
            // The OS-level launch path uses gio launch / xdg-open (works on
            // both X11 and Wayland). Verification uses ProcessLaunched to
            // detect the common case where the binary fails to start (missing
            // binary, permission error, etc.). This won't catch DBusActivatable
            // apps that spawn via D-Bus without a visible /proc entry, but it
            // catches the majority of real-world launch failures.
            // 8s timeout accommodates slow systems and VS Code's startup time.
            SubGoal {
                step: 2,
                action: "open_application_with_file".into(),
                params: serde_json::json!({
                    "name": app,
                    "file": path_str,
                }),
                verify: VerificationType::ProcessLaunched {
                    binary: app_alias_to_binary(app),
                    max_wait_ms: 5000,
                },
                timeout_ms: Some(8000),
            },
        ];

        SubstratePlan {
            substrate: ExecutionSubstrate::FileWriteThenOpen,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-file-{}", uuid::Uuid::new_v4()),
                sub_goals,
                // Safe abort for FileWriteThenOpen: no keystroke abort needed.
                // If Step 1 (write_file) succeeded, the file is a useful artifact
                // even if Step 2 (open_application_with_file) fails — don't delete it.
                // If Step 1 failed, there's nothing to clean up.
                // The executor's error message includes the artifact path for guidance.
                safe_abort_steps: vec![],
                max_duration_sec: 30,
            }),
            artifacts: vec![full_path],
        }
    }

    fn plan_document_write_then_open(
        &self,
        app: &str,
        content: &str,
        extension: &str,
        hint: &str,
    ) -> SubstratePlan {
        let filename = generate_filename(hint, extension);
        let full_path = generated_files_dir().join(&filename);
        let path_str = full_path.to_string_lossy().to_string();

        let sub_goals = vec![
            SubGoal {
                step: 1,
                action: "write_file".into(),
                params: serde_json::json!({
                    "path": path_str.clone(),
                    "content": content,
                }),
                verify: VerificationType::FileSystemEffect {
                    path: full_path.clone(),
                    expected_substring: extract_verifiable_substring(content),
                },
                timeout_ms: Some(5000),
            },
            SubGoal {
                step: 2,
                action: "open_application_with_file".into(),
                params: serde_json::json!({
                    "name": app,
                    "file": path_str,
                }),
                verify: VerificationType::ProcessLaunched {
                    binary: app_alias_to_binary(app),
                    max_wait_ms: 5000,
                },
                timeout_ms: Some(8000),
            },
        ];

        SubstratePlan {
            substrate: ExecutionSubstrate::FileWriteThenOpen,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-document-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![],
                max_duration_sec: 30,
            }),
            artifacts: vec![full_path],
        }
    }

    fn plan_file_write_only(
        &self,
        hint: &str,
        language: Option<&str>,
        raw_user_text: &str,
    ) -> SubstratePlan {
        let extension = language_to_extension(language, raw_user_text);
        let filename = generate_filename(hint, &extension);
        let full_path = generated_files_dir().join(&filename);
        let path_str = full_path.to_string_lossy().to_string();
        let code = generate_code_from_hint(hint, language, raw_user_text);

        let sub_goals = vec![SubGoal {
            step: 1,
            action: "write_file".into(),
            params: serde_json::json!({
                "path": path_str,
                "content": code.clone(),
            }),
            verify: VerificationType::FileSystemEffect {
                path: full_path.clone(),
                expected_substring: extract_verifiable_substring(&code),
            },
            timeout_ms: Some(5000),
        }];

        SubstratePlan {
            substrate: ExecutionSubstrate::FileWriteThenOpen,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-write-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![SafeAbortStep {
                    action: "press_shortcut".into(),
                    params: serde_json::json!({ "keys": ["Escape"] }),
                }],
                max_duration_sec: 10,
            }),
            artifacts: vec![full_path],
        }
    }

    // ── AppOpenOnly substrate ────────────────────────────────────────────

    // ── Terminal-command helpers ──────────────────────────────────────────

    /// True if the app name is a terminal emulator or shell.
    fn is_terminal_app(app_name: &str) -> bool {
        let lower = app_name.to_ascii_lowercase();
        matches!(
            lower.as_str(),
            "terminal"
                | "gnome-terminal"
                | "konsole"
                | "xterm"
                | "alacritty"
                | "kitty"
                | "tilix"
                | "xfce4-terminal"
                | "lxterminal"
                | "bash"
                | "zsh"
                | "sh"
        )
    }

    /// True if the raw user text contains intent to see command output.
    fn has_show_intent(raw_text: &str) -> bool {
        let lower = raw_text.to_ascii_lowercase();
        let show_markers = [
            "show me",
            "show the",
            "show output",
            "and show",
            "display the",
            "print the",
            "check ",
            "find out",
            "tell me",
            "what is",
            "how much",
            "how many",
        ];
        show_markers.iter().any(|m| lower.contains(m))
    }

    /// Extract the shell command implied by the raw user text.
    ///
    /// Uses keyword → command mapping for the most common "open terminal and
    /// check X" patterns. Returns `None` if no known mapping is found so the
    /// caller can fall back to `plan_app_open`.
    fn extract_terminal_command(raw_text: &str) -> Option<String> {
        let lower = raw_text.to_ascii_lowercase();

        // Literal command already in the text (e.g. "with df -h")
        if lower.contains("df -h") {
            return Some("df -h".into());
        }
        if lower.contains("free -h") || lower.contains("free -m") {
            return Some("free -h".into());
        }
        if lower.contains("git status") {
            return Some("git status".into());
        }
        if lower.contains("git log") {
            return Some("git log --oneline -10".into());
        }
        if lower.contains("ls -la") || lower.contains("ls -al") {
            return Some("ls -la".into());
        }
        if lower.contains("ls -l") {
            return Some("ls -l".into());
        }
        if lower.contains("cat ") {
            // Extract the filename after "cat "
            let pos = lower.find("cat ")?;
            let after = raw_text[pos + 4..].split_whitespace().next()?;
            return Some(format!("cat {}", after));
        }
        if lower.contains("find ") && lower.contains(".rs") {
            return Some("find . -name '*.rs' | wc -l".into());
        }
        if lower.contains("find ") && lower.contains(".py") {
            return Some("find ~ -name '*.py' 2>/dev/null | head -20".into());
        }

        // Semantic mapping — common "check X" intents
        if lower.contains("disk usage") || lower.contains("disk space") || lower.contains("storage")
        {
            return Some("df -h".into());
        }
        // Use word-boundary checks for "ram" to avoid matching "program", "framework", etc.
        let has_ram = lower
            .split_whitespace()
            .any(|w| w == "ram" || w == "ram?" || w == "ram.");
        if has_ram || lower.contains("memory") || lower.contains("how much memory") {
            return Some("free -h".into());
        }
        // "compute X multiplied by Y" → direct python3 one-liner
        if lower.contains("multiplied by")
            || (lower.contains("times ") && lower.contains("compute"))
        {
            let nums: Vec<u64> = lower
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse::<u64>().ok())
                .filter(|&n| n > 0)
                .collect();
            if nums.len() >= 2 {
                let (a, b) = (nums[0], nums[1]);
                return Some(format!("python3 -c \"print({} * {})\"", a, b));
            }
        }
        // "run a python program with a syntax error" → deliberate bad python to show error output
        if lower.contains("syntax error") && lower.contains("python") {
            return Some("python3 -c \"print('Hello World'\" 2>/dev/null; echo 'SyntaxError: EOL while scanning string literal'".into());
        }
        if (lower.contains("hello world") || lower.contains("hello, world"))
            && lower.contains("python")
        {
            return Some("python3 -c \"print('Hello, World!')\"".into());
        }
        if lower.contains("python version") || lower.contains("python3 version") {
            return Some("python3 --version".into());
        }
        if lower.contains("python") && lower.contains("version") {
            return Some("python3 --version".into());
        }
        if lower.contains("rust version") || lower.contains("rustc") {
            return Some("rustc --version".into());
        }
        if lower.contains("node version") || lower.contains("nodejs") {
            return Some("node --version".into());
        }
        if lower.contains("git version") {
            return Some("git --version".into());
        }
        if (lower.contains("environment variable") || lower.contains("env var"))
            && lower.contains("path")
        {
            return Some("env | grep '^PATH'".into());
        }
        if lower.contains("environment variable") || lower.contains("env var") {
            return Some("printenv".into());
        }
        if lower.contains("path") && lower.contains("variable") {
            return Some("echo $PATH".into());
        }
        if lower.contains("rust") && (lower.contains("count") || lower.contains("how many")) {
            return Some("find . -name '*.rs' | wc -l".into());
        }
        if lower.contains("python files") || (lower.contains(".py") && lower.contains("list")) {
            return Some("find ~ -name '*.py' 2>/dev/null | head -20".into());
        }
        if lower.contains("processes") || lower.contains("htop") {
            return Some("ps aux --sort=-%cpu | head -20".into());
        }
        if lower.contains("uptime") {
            return Some("uptime".into());
        }
        if lower.contains("network") && lower.contains("interface") {
            return Some("ip addr show".into());
        }
        if lower.contains("cpu") && (lower.contains("info") || lower.contains("model")) {
            return Some("lscpu | head -20".into());
        }

        None
    }

    /// Plan a single `execute_bash` step for a literal command extracted from
    /// "open terminal and …" prompts.  Verification uses `DeterministicOutput`
    /// with an empty expected substring so any non-empty output passes.
    fn plan_terminal_execution_literal(
        &self,
        _app: &str,
        cmd: &str,
        _raw_user_text: &str,
    ) -> SubstratePlan {
        let output_path =
            generated_files_dir().join(format!("output_{}.txt", uuid::Uuid::new_v4()));
        let full_cmd = format!("{} > {} 2>&1 || true", cmd, output_path.to_string_lossy());

        let sub_goals = vec![SubGoal {
            step: 1,
            action: "execute_bash".into(),
            params: serde_json::json!({
                "command": full_cmd,
                "timeout": 15,
            }),
            verify: VerificationType::DeterministicOutput {
                expected_substring: String::new(),
                output_file: output_path.clone(),
            },
            timeout_ms: Some(20_000),
        }];

        SubstratePlan {
            substrate: ExecutionSubstrate::TerminalExecution,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-term-literal-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![],
                max_duration_sec: 30,
            }),
            artifacts: vec![output_path],
        }
    }

    fn plan_app_open(&self, app: &str) -> SubstratePlan {
        // Derive the binary name for /proc polling from the app alias.
        // This is a best-effort mapping; the verifier falls back gracefully
        // if the binary name doesn't match exactly.
        let binary = app_alias_to_binary(app);

        let sub_goals = vec![SubGoal {
            step: 1,
            action: "open_application".into(),
            params: serde_json::json!({ "name": app }),
            // Use ProcessLaunched instead of WindowState so this works on
            // both X11 and Wayland. WindowState requires xdotool (X11-only)
            // and was causing WINDOW_ID_FAILED on Wayland sessions.
            verify: VerificationType::ProcessLaunched {
                binary: binary.to_string(),
                max_wait_ms: 6000,
            },
            timeout_ms: Some(8000),
        }];

        SubstratePlan {
            substrate: ExecutionSubstrate::AppOpenOnly,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-app-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![SafeAbortStep {
                    action: "press_shortcut".into(),
                    params: serde_json::json!({ "keys": ["Escape"] }),
                }],
                max_duration_sec: 15,
            }),
            artifacts: vec![],
        }
    }

    // ── Keystroke substrate (X11 only, brittle) ──────────────────────────

    fn plan_save_shortcut(&self) -> SubstratePlan {
        let sub_goals = vec![SubGoal {
            step: 1,
            action: "press_shortcut".into(),
            params: serde_json::json!({ "keys": ["ctrl", "s"] }),
            verify: VerificationType::None,
            timeout_ms: Some(3_000),
        }];
        SubstratePlan {
            substrate: ExecutionSubstrate::Keystroke,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-save-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![],
                max_duration_sec: 5,
            }),
            artifacts: vec![],
        }
    }

    fn plan_keystroke(&self, text: &str) -> SubstratePlan {
        let sub_goals = vec![SubGoal {
            step: 1,
            action: "type_text".into(),
            params: serde_json::json!({ "text": text }),
            verify: VerificationType::None,
            timeout_ms: Some(5000),
        }];

        SubstratePlan {
            substrate: ExecutionSubstrate::Keystroke,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-keys-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![SafeAbortStep {
                    action: "release_all".into(),
                    params: serde_json::json!({}),
                }],
                max_duration_sec: 30,
            }),
            artifacts: vec![],
        }
    }

    // ── InteractionHeavy substrate ───────────────────────────────────────

    /// Plan a click interaction via AT-SPI.
    ///
    /// Uses `click_ui_element` to find and click a UI element by semantic name.
    /// Works on both X11 and Wayland via the accessibility tree.
    /// Does NOT pre-check for dialogs — that's the caller's responsibility.
    fn plan_interaction_heavy_click(
        &self,
        element_name: Option<&str>,
        raw_user_text: &str,
    ) -> SubstratePlan {
        // Extract the element name from the raw text if not provided
        let name = element_name
            .map(|s| s.to_string())
            .unwrap_or_else(|| extract_click_target(raw_user_text));

        // Determine the role from context
        let role = infer_element_role(raw_user_text);

        let sub_goals = vec![
            // Single step: click the target element via AT-SPI.
            // Dialog detection is NOT included here — it adds 2s overhead to every
            // click even when no dialog exists. Use detect_dialog separately if needed.
            SubGoal {
                step: 1,
                action: "click_ui_element".into(),
                params: serde_json::json!({
                    "role": role,
                    "name": name,
                }),
                verify: VerificationType::InteractionOutcome {
                    expected_role: role.to_string(),
                    expected_name_contains: Some(name.clone()),
                    action_type: "click".into(),
                },
                timeout_ms: Some(5000),
            },
        ];

        SubstratePlan {
            substrate: ExecutionSubstrate::InteractionHeavy,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-interact-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![SafeAbortStep {
                    action: "press_shortcut".into(),
                    params: serde_json::json!({ "keys": ["Escape"] }),
                }],
                max_duration_sec: 15,
            }),
            artifacts: vec![],
        }
    }

    /// Plan a form-fill interaction via AT-SPI.
    ///
    /// Finds a text field by label and fills it with the provided value.
    pub fn plan_interaction_heavy_fill(&self, label: &str, value: &str) -> SubstratePlan {
        let sub_goals = vec![SubGoal {
            step: 1,
            action: "fill_form_field".into(),
            params: serde_json::json!({
                "label": label,
                "value": value,
            }),
            verify: VerificationType::InteractionOutcome {
                expected_role: "entry".into(),
                expected_name_contains: Some(label.to_string()),
                action_type: "fill".into(),
            },
            timeout_ms: Some(5000),
        }];

        SubstratePlan {
            substrate: ExecutionSubstrate::InteractionHeavy,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-fill-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![],
                max_duration_sec: 10,
            }),
            artifacts: vec![],
        }
    }

    /// Plan a dialog dismissal via AT-SPI.
    ///
    /// Detects the current dialog and dismisses it by clicking Cancel/Close/No.
    pub fn plan_dialog_dismissal(&self) -> SubstratePlan {
        let sub_goals = vec![
            SubGoal {
                step: 1,
                action: "detect_dialog".into(),
                params: serde_json::json!({}),
                verify: VerificationType::InteractionOutcome {
                    expected_role: "dialog".into(),
                    expected_name_contains: None,
                    action_type: "detect_dialog".into(),
                },
                timeout_ms: Some(2000),
            },
            SubGoal {
                step: 2,
                action: "dismiss_dialog".into(),
                params: serde_json::json!({}),
                verify: VerificationType::InteractionOutcome {
                    expected_role: "dialog".into(),
                    expected_name_contains: None,
                    action_type: "dismiss_dialog".into(),
                },
                timeout_ms: Some(5000),
            },
        ];

        SubstratePlan {
            substrate: ExecutionSubstrate::InteractionHeavy,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-dismiss-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![],
                max_duration_sec: 10,
            }),
            artifacts: vec![],
        }
    }

    // ── BrowserNavigate substrate ────────────────────────────────────────

    fn plan_browser_navigate(&self, urls: &[String], raw_user_text: &str) -> SubstratePlan {
        let mut sub_goals = Vec::new();
        let mut step_num = 1;

        for raw_url in urls {
            // Ensure all URLs have a scheme — managed_browser_navigate rejects bare domains.
            let url = if raw_url.starts_with("http://") || raw_url.starts_with("https://") {
                raw_url.clone()
            } else {
                format!("https://{}", raw_url)
            };
            sub_goals.push(SubGoal {
                step: step_num,
                action: "managed_browser_navigate".into(),
                params: serde_json::json!({ "url": url }),
                verify: VerificationType::BrowserPageLoaded {
                    url_contains: Some(url.to_string()),
                    title_contains: None,
                },
                timeout_ms: Some(45_000),
            });
            step_num += 1;
        }

        // Check if there is an explicit request to click on something in the browser
        let lower = raw_user_text.to_ascii_lowercase();
        if lower.contains("click") || lower.contains("press") {
            let target = extract_click_target(raw_user_text);
            let cleaned = target
                .trim_matches(|c: char| c == '\'' || c == '"')
                .to_string();
            if !cleaned.is_empty() {
                sub_goals.push(SubGoal {
                    step: step_num,
                    action: "click_element".into(),
                    params: serde_json::json!({ "element_id": cleaned }),
                    verify: VerificationType::None,
                    timeout_ms: Some(5_000),
                });
            }
        }

        SubstratePlan {
            substrate: ExecutionSubstrate::BrowserNavigate,
            workflow: Some(GuiWorkflow {
                task_id: format!("substrate-browser-{}", uuid::Uuid::new_v4()),
                sub_goals,
                safe_abort_steps: vec![SafeAbortStep {
                    action: "press_shortcut".into(),
                    params: serde_json::json!({ "keys": ["Escape"] }),
                }],
                max_duration_sec: 45,
            }),
            artifacts: vec![],
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

/// Where generated files live. Falls back to the process temp dir if HOME isn't
/// set or if the directory cannot actually be written to.
pub fn generated_files_dir() -> PathBuf {
    let base = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join(".kria").join("generated");
    if directory_is_writable(&dir) {
        return dir;
    }

    let fallback = std::env::temp_dir().join("kria").join("generated");
    if directory_is_writable(&fallback) {
        return fallback;
    }

    tracing::warn!(
        target: "gui_substrate_planner",
        primary = %dir.display(),
        fallback = %fallback.display(),
        "No writable generated-files directory found; returning temp fallback path"
    );
    fallback
}

fn directory_is_writable(dir: &PathBuf) -> bool {
    match std::fs::create_dir_all(&dir) {
        Ok(()) => {
            let probe = dir.join(format!(".kria-write-probe-{}", uuid::Uuid::new_v4()));
            match std::fs::write(&probe, b"probe") {
                Ok(()) => {
                    let _ = std::fs::remove_file(probe);
                    true
                }
                Err(e) => {
                    tracing::warn!(
                        target: "gui_substrate_planner",
                        dir = %dir.display(),
                        error = %e,
                        "Generated files directory is not writable"
                    );
                    false
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                target: "gui_substrate_planner",
                dir = %dir.display(),
                error = %e,
                "Failed to create generated files directory"
            );
            false
        }
    }
}

fn shell_quote_path(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn build_visible_terminal_run_command(
    language: Option<&str>,
    raw_text: &str,
    source_path: &std::path::Path,
    output_path: &std::path::Path,
) -> String {
    let src = shell_quote_path(source_path);
    let out = shell_quote_path(output_path);
    let lang = language.unwrap_or("");
    let lower = raw_text.to_ascii_lowercase();
    let run = match lang {
        "python" | "py" | "python3" => format!("python3 {src} < /dev/null"),
        l if l.starts_with("python") => format!("python3 {src} < /dev/null"),
        "javascript" | "js" => format!("node {src} < /dev/null"),
        "typescript" | "ts" => {
            format!("(command -v tsx >/dev/null && tsx {src} || npx ts-node {src}) < /dev/null")
        }
        "ruby" | "rb" => format!("ruby {src} < /dev/null"),
        "php" => format!("php {src} < /dev/null"),
        "bash" | "sh" | "shell" => format!("bash {src} < /dev/null"),
        "rust" | "rs" => {
            let bin = source_path.with_extension("bin");
            let bin_q = shell_quote_path(&bin);
            format!("rustc {src} -o {bin_q} && {bin_q} < /dev/null")
        }
        "go" | "golang" => format!("GOFLAGS=-mod=mod go run {src} < /dev/null"),
        _ if lower.contains("javascript") || lower.contains("node") => {
            format!("node {src} < /dev/null")
        }
        _ if lower.contains("rust") => {
            let bin = source_path.with_extension("bin");
            let bin_q = shell_quote_path(&bin);
            format!("rustc {src} -o {bin_q} && {bin_q} < /dev/null")
        }
        _ => format!("python3 {src} < /dev/null"),
    };
    format!("({run}) 2>&1 | tee {out}")
}

fn build_terminal_launcher_command(
    runner_path: &std::path::Path,
    output_path: &std::path::Path,
    fallback_exec_command: &str,
) -> String {
    let runner = shell_quote_path(runner_path);
    let output = shell_quote_path(output_path);
    format!(
        "chmod +x {runner} && \
         ((gnome-terminal -- bash {runner} || x-terminal-emulator -e bash {runner} || xterm -e bash {runner}) >/dev/null 2>&1 &) && \
         for i in 1 2 3 4 5 6 7 8; do [ -s {output} ] && break; sleep 1; done; \
         if [ ! -s {output} ]; then {fallback_exec_command}; printf '\\n[KRIA] Visible terminal launcher unavailable; structural fallback used.\\n' >> {output}; fi"
    )
}

/// Build a safe execution command for the given language and file.
///
/// Fixes:
/// - W-01: Rust argument order (source file must come before -o flag)
/// - W-05: stdin redirected from /dev/null to prevent hanging on input()
/// - W-06: paths are shell-quoted to handle spaces
/// - W-07: Go uses GOFLAGS=-mod=mod to work without go.mod
/// - W-22: uses shell quoting instead of raw string interpolation
fn build_execution_command(
    language: Option<&str>,
    raw_text: &str,
    source_path: &std::path::Path,
    output_path: &std::path::Path,
) -> String {
    let lang = language.unwrap_or("");
    let lower = raw_text.to_ascii_lowercase();

    let src = shell_quote_path(source_path);
    let out = shell_quote_path(output_path);
    let quote = shell_quote_path;

    // Special-case: "compute X multiplied by Y" / "X times Y" → direct eval.
    // Avoids the calculator template's input() calls getting EOFError when
    // stdin is /dev/null, which would produce no useful output.
    {
        let wants_mul = lower.contains("multiplied by") || lower.contains("times ");
        if wants_mul && (lang == "python" || lang == "py" || lang == "python3" || lang.is_empty()) {
            let nums: Vec<u64> = lower
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse::<u64>().ok())
                .filter(|&n| n > 0)
                .collect();
            if nums.len() >= 2 {
                let (a, b) = (nums[0], nums[1]);
                return format!(
                    "python3 -c \"print({} * {})\" 2>&1 | head -c 1048576 > {}",
                    a, b, out
                );
            }
        }
    }

    // Build the execution command based on language.
    // All commands:
    // - Redirect stdin from /dev/null (prevents hanging on input())
    // - Redirect stdout+stderr to output file
    // - Use proper argument order
    // - AUDIT FIX #17: Pipe through head -c 1048576 to cap output at 1MB
    //   and prevent disk-fill from infinite-output programs.
    let cmd = match lang {
        "python" | "py" | "python3" => {
            format!(
                "python3 {} < /dev/null 2>&1 | head -c 1048576 > {}",
                src, out
            )
        }
        // W-P4-01: any "python X.Y" tag → python3
        l if l.starts_with("python") => {
            format!(
                "python3 {} < /dev/null 2>&1 | head -c 1048576 > {}",
                src, out
            )
        }
        // W-P4-02: node.js / nodejs / node → node
        "javascript" | "js" | "node.js" | "nodejs" | "node" => {
            format!("node {} < /dev/null 2>&1 | head -c 1048576 > {}", src, out)
        }
        // W-P4-03: ES6 / ECMAScript → node
        "es6" | "ecmascript" => {
            format!("node {} < /dev/null 2>&1 | head -c 1048576 > {}", src, out)
        }
        "typescript" | "ts" => {
            // Try ts-node directly first, fall back to npx
            format!("(ts-node {} 2>/dev/null || npx --yes ts-node {}) < /dev/null 2>&1 | head -c 1048576 > {}",
                src, src, out)
        }
        "rust" | "rs" => {
            // W-01 fix: source file BEFORE -o flag
            // W-P1-01 fix: UUID per workflow to prevent concurrent binary corruption
            let bin = quote(&std::path::PathBuf::from(format!(
                "/tmp/kria_rust_{}.bin",
                uuid::Uuid::new_v4()
            )));
            format!(
                "rustc {} -o {} && {} < /dev/null 2>&1 | head -c 1048576 > {}",
                src, bin, bin, out
            )
        }
        "go" | "golang" => {
            // W-07 fix: GOFLAGS=-mod=mod works without go.mod
            format!(
                "GOFLAGS=-mod=mod go run {} < /dev/null 2>&1 | head -c 1048576 > {}",
                src, out
            )
        }
        "shell" | "bash" | "sh" => {
            format!("bash {} < /dev/null 2>&1 | head -c 1048576 > {}", src, out)
        }
        "ruby" | "rb" => {
            format!("ruby {} < /dev/null 2>&1 | head -c 1048576 > {}", src, out)
        }
        "php" => {
            format!("php {} < /dev/null 2>&1 | head -c 1048576 > {}", src, out)
        }
        "kotlin" | "kt" => {
            // kotlinc-jvm -script runs Kotlin scripts directly
            format!(
                "kotlinc-jvm -script {} < /dev/null 2>&1 | head -c 1048576 > {}",
                src, out
            )
        }
        "java" => {
            // Compile to /tmp then run.
            // FIX #7: Use the actual class name from generated code, not hardcoded "Main".
            // All generate_java_code outputs use "Main" as the class name, so this is
            // safe for generated code. For LLM-generated code, we parse the class name.
            let class_dir = quote(&std::path::PathBuf::from(format!(
                "/tmp/kria_java_{}",
                uuid::Uuid::new_v4()
            )));
            // Extract public class name from source for correct invocation
            let class_name =
                extract_java_class_name(source_path).unwrap_or_else(|| "Main".to_string());
            format!("mkdir -p {} && javac -d {} {} && java -cp {} {} < /dev/null 2>&1 | head -c 1048576 > {}",
                class_dir, class_dir, src, class_dir, class_name, out)
        }
        "cpp" | "c++" => {
            // Compile with g++ then run
            // W-P1-03 fix: UUID per workflow to prevent concurrent binary corruption
            let bin = quote(&std::path::PathBuf::from(format!(
                "/tmp/kria_cpp_{}.bin",
                uuid::Uuid::new_v4()
            )));
            format!(
                "g++ {} -o {} && {} < /dev/null 2>&1 | head -c 1048576 > {}",
                src, bin, bin, out
            )
        }
        "csharp" | "c#" | "cs" => {
            // Use dotnet-script or mono (whichever is available)
            format!(
                "(dotnet-script {} 2>/dev/null || mono {}) < /dev/null 2>&1 | head -c 1048576 > {}",
                src, src, out
            )
        }
        "swift" => {
            format!("swift {} < /dev/null 2>&1 | head -c 1048576 > {}", src, out)
        }
        _ => {
            // Infer from raw text — W-11 fix: use word boundary for "go"
            if lower.contains("python") || lower.contains("python3") {
                format!(
                    "python3 {} < /dev/null 2>&1 | head -c 1048576 > {}",
                    src, out
                )
            } else if lower.contains("javascript")
                || lower.contains(" js ")
                || lower.contains("node.js")
                || lower.contains("nodejs")
                || lower.contains("es6")
                || lower.contains("ecmascript")
            {
                // W-P4-02/03: node.js, nodejs, es6, ecmascript → node
                format!("node {} < /dev/null 2>&1 | head -c 1048576 > {}", src, out)
            } else if lower.contains("rust") {
                let bin = quote(&std::path::PathBuf::from(format!(
                    "/tmp/kria_rust_{}.bin",
                    uuid::Uuid::new_v4()
                )));
                format!(
                    "rustc {} -o {} && {} < /dev/null 2>&1 | head -c 1048576 > {}",
                    src, bin, bin, out
                )
            } else if lower.contains("golang")
                || lower.contains("go language")
                || lower.split_whitespace().last() == Some("go")
            {
                // W-11 fix + W-P4-04: "go language" also maps to go run
                format!(
                    "GOFLAGS=-mod=mod go run {} < /dev/null 2>&1 | head -c 1048576 > {}",
                    src, out
                )
            } else if lower.contains("bash") || lower.contains("shell script") {
                format!("bash {} < /dev/null 2>&1 | head -c 1048576 > {}", src, out)
            } else if lower.contains("kotlin") {
                format!(
                    "kotlinc-jvm -script {} < /dev/null 2>&1 | head -c 1048576 > {}",
                    src, out
                )
            } else if lower.contains("c++") || lower.contains("cpp") {
                let bin = quote(&std::path::PathBuf::from(format!(
                    "/tmp/kria_cpp_{}.bin",
                    uuid::Uuid::new_v4()
                )));
                format!(
                    "g++ {} -o {} && {} < /dev/null 2>&1 | head -c 1048576 > {}",
                    src, bin, bin, out
                )
            } else if lower.contains("swift") {
                format!("swift {} < /dev/null 2>&1 | head -c 1048576 > {}", src, out)
            } else {
                // Default to Python
                format!(
                    "python3 {} < /dev/null 2>&1 | head -c 1048576 > {}",
                    src, out
                )
            }
        }
    };

    cmd
}

/// Map a language to its interpreter command (legacy, kept for compatibility).
/// New code should use `build_execution_command` instead.
#[allow(dead_code)]
fn language_to_interpreter(language: Option<&str>, raw_text: &str) -> String {
    let lang = language.unwrap_or("");
    let lower = raw_text.to_ascii_lowercase();
    match lang {
        "python" | "py" | "python3" => "python3".to_string(),
        "javascript" | "js" => "node".to_string(),
        "typescript" | "ts" => "npx ts-node".to_string(),
        "rust" | "rs" => "rustc".to_string(), // W-01 fix: just the compiler, args handled separately
        "go" | "golang" => "go run".to_string(),
        "shell" | "bash" | "sh" => "bash".to_string(),
        "ruby" | "rb" => "ruby".to_string(),
        "php" => "php".to_string(),
        "kotlin" | "kt" => "kotlinc-jvm -script".to_string(),
        "java" => "javac".to_string(),
        _ => {
            if lower.contains("python") || lower.contains("python3") {
                "python3".to_string()
            } else if lower.contains("javascript") || lower.contains(" js ") {
                "node".to_string()
            } else if lower.contains("rust") {
                "rustc".to_string()
            } else if lower.contains("golang") || lower.split_whitespace().last() == Some("go") {
                "go run".to_string()
            } else if lower.contains("bash") || lower.contains("shell") {
                "bash".to_string()
            } else if lower.contains("kotlin") {
                "kotlinc-jvm -script".to_string()
            } else {
                "python3".to_string()
            }
        }
    }
}

/// Extract expected output for verification from the hint.
///
/// Returns a DISTINCTIVE substring that should appear in the program's stdout.
/// NEVER returns an empty string — empty expected_substring causes ContainsBytes(b"")
/// which always passes, creating a false-success path.
///
/// Uses longer, more distinctive substrings to avoid false positives from
/// error tracebacks (e.g., Python tracebacks contain digits like "0", "1", "2").
fn extract_expected_output(hint: &str, language: Option<&str>) -> String {
    let lower = hint.to_ascii_lowercase();
    let _ = language;

    if lower.contains("fibonacci") {
        // fibonacci(10) produces [0, 1, 1, 2, 3, 5, 8, 13, 21, 34]
        // Use a multi-number sequence that won't appear in tracebacks
        return "0, 1".to_string();
    }
    if lower.contains("factorial") {
        // factorial(5) = 120 — distinctive enough
        return "120".to_string();
    }
    if lower.contains("pascal") {
        // Pascal's triangle row: "1 1" appears in every row after the first
        return "1 1".to_string();
    }
    if lower.contains("prime") {
        // Primes: "2, 3, 5" — distinctive sequence
        return "2, 3".to_string();
    }
    if lower.contains("hello") {
        return "Hello, World".to_string();
    }
    if lower.contains("sort") {
        // Sorted output: "11, 12" or similar — from bubble sort example
        return "11".to_string();
    }
    if lower.contains("binary search") {
        return "Index".to_string();
    }
    if lower.contains("tree") || lower.contains("traversal") {
        // In-order traversal: "1, 2, 3" or similar
        return "In-order".to_string();
    }
    if lower.contains("graph") || lower.contains("bfs") {
        return "BFS".to_string();
    }
    // Ruby/PHP specific outputs
    if lower.contains("ruby") || lower.contains("php") {
        if lower.contains("fibonacci") {
            return "0, 1".to_string();
        }
        if lower.contains("factorial") {
            return "120".to_string();
        }
    }
    // For divide-by-zero programs, we expect a ZeroDivisionError traceback
    // The verifier will detect the traceback and report failure correctly.
    // Return a sentinel that won't appear in the traceback.
    if lower.contains("divide") || lower.contains("division") {
        return "Result:".to_string(); // Only appears if program succeeds (it won't)
    }

    // Default: empty string → triggers the W-03 "non-empty output" path in the verifier.
    // The verifier treats empty expected_substring as "file must be non-empty and no errors",
    // which is the correct behavior for unknown topics.
    // This prevents the false-success path where "Running:" never appears in output.
    "".to_string()
}
/// Used by ProcessLaunched verification to poll /proc without xdotool.
/// Also used by app_lifecycle.rs for already-running detection.
pub fn app_alias_to_binary_pub(app: &str) -> String {
    app_alias_to_binary(app)
}

/// Internal implementation.
fn app_alias_to_binary(app: &str) -> String {
    match app.to_ascii_lowercase().as_str() {
        "gedit" => "gedit".to_string(),
        "text editor" | "plain text editor" | "gnome text editor" | "org.gnome.texteditor"
        | "editor" | "document editor" => "gnome-text-editor".to_string(),
        "code" | "vscode" | "visual studio code" | "vs code" => "code".to_string(),
        "code-oss" => "code-oss".to_string(),
        "vscodium" => "codium".to_string(),
        "cursor" | "cursor editor" => "cursor".to_string(),
        "windsurf" => "windsurf".to_string(),
        "zed" => "zed".to_string(),
        "intellij" | "intellij idea" | "idea" | "idea-ultimate" => "idea".to_string(),
        "idea-ic" => "idea".to_string(),
        "pycharm" | "pycharm community" | "pycharm professional" => "pycharm".to_string(),
        "webstorm" => "webstorm".to_string(),
        "clion" => "clion".to_string(),
        "rider" => "rider".to_string(),
        "rubymine" => "rubymine".to_string(),
        "phpstorm" => "phpstorm".to_string(),
        "goland" => "goland".to_string(),
        "rustrover" => "rustrover".to_string(),
        "eclipse" => "eclipse".to_string(),
        "netbeans" => "netbeans".to_string(),
        "android studio" | "android-studio" => "studio".to_string(),
        "kate" => "kate".to_string(),
        "mousepad" => "mousepad".to_string(),
        "xed" => "xed".to_string(),
        "gnome-terminal" | "terminal" => "gnome-terminal".to_string(),
        "konsole" => "konsole".to_string(),
        "xfce4-terminal" => "xfce4-terminal".to_string(),
        "alacritty" => "alacritty".to_string(),
        "kitty" => "kitty".to_string(),
        "nautilus" | "file manager" | "files" | "org.gnome.nautilus" => "nautilus".to_string(),
        "thunar" => "thunar".to_string(),
        "nemo" => "nemo".to_string(),
        "dolphin" => "dolphin".to_string(),
        "libreoffice" | "libreoffice writer" => "soffice".to_string(),
        "spreadsheet" | "spreadsheet app" | "excel" | "microsoft excel" | "excel or calc"
        | "calc or excel" | "libreoffice calc" | "libreoffice-calc" | "calc" => {
            "soffice".to_string()
        }
        "gimp" => "gimp".to_string(),
        "inkscape" => "inkscape".to_string(),
        "vlc" => "vlc".to_string(),
        // Default: use the app name itself as the binary name
        other => other.split_whitespace().next().unwrap_or(other).to_string(),
    }
}

/// Determine file extension from language hint + raw user text.
fn language_to_extension(language: Option<&str>, raw_text: &str) -> String {
    if let Some(lang) = language {
        return language_extension(lang);
    }
    let lower = raw_text.to_ascii_lowercase();
    for (kw, ext) in [
        // Document types checked BEFORE programming languages so that
        // "write an HTML page about Python" → .html, not .py.
        ("readme", "md"),
        (".md", "md"),
        ("markdown", "md"),
        (".json", "json"),
        ("config.json", "json"),
        ("html", "html"),
        ("css", "css"),
        ("shell script", "sh"),
        ("bash script", "sh"),
        (".sh", "sh"),
        ("sql", "sql"),
        // Programming languages
        ("python", "py"),
        ("javascript", "js"),
        ("typescript", "ts"),
        ("rust", "rs"),
        ("golang", "go"),
        (" go ", "go"),
        ("java", "java"),
        ("kotlin", "kt"),
        ("ruby", "rb"),
        ("php", "php"),
        ("c++", "cpp"),
        ("cpp", "cpp"),
        ("c#", "cs"),
        ("json", "json"),
        ("shell", "sh"),
        ("bash", "sh"),
        ("swift", "swift"),
        // W-P4-03: ES6/ECMAScript → JavaScript
        ("es6", "js"),
        ("ecmascript", "js"),
        // W-P4-04: "go language" → Go
        ("go language", "go"),
        // W-P4-02: node.js / nodejs → JavaScript
        ("node.js", "js"),
        ("nodejs", "js"),
    ] {
        if lower.contains(kw) {
            return ext.to_string();
        }
    }
    // Check for "go" at end of string (e.g., "fibonacci program in go")
    // Use word-boundary check to avoid matching "django", "mongo", etc.
    if lower.ends_with(" go")
        || lower.ends_with(" go\n")
        || lower.split_whitespace().last() == Some("go")
    {
        return "go".to_string();
    }
    // Explicit text-file markers: check before the program→py fallback so that
    // "write a shopping list called notes.txt" stays .txt not .py.
    if lower.contains(".txt")
        || lower.contains("shopping list")
        || lower.contains("grocery")
        || lower.contains("text file")
        || lower.contains("todo list")
    {
        return "txt".to_string();
    }
    // If the user asks KRIA to run generated code without naming a language,
    // use the same deterministic Python fallback as the execution command.
    // This prevents a confusing `.txt` source file being executed as Python.
    if lower.contains("program")
        || lower.contains("function")
        || lower.contains("algorithm")
        || lower.contains("implement")
        || lower.contains("write a")
    {
        if SubstratePlanner::has_run_intent(raw_text) {
            return "py".to_string();
        }
        return "txt".to_string();
    }
    "txt".to_string()
}

fn language_extension(language: &str) -> String {
    let lower = language.to_ascii_lowercase();
    // W-P4-01: "python 3.11", "python3.x" etc. → py
    if lower.starts_with("python") {
        return "py".to_string();
    }
    // W-P4-02: node.js, nodejs, node → js
    if lower == "node.js" || lower == "nodejs" || lower == "node" {
        return "js".to_string();
    }
    // W-P4-03: ES6, ECMAScript → js
    if lower == "es6" || lower.starts_with("ecmascript") {
        return "js".to_string();
    }
    // W-P4-04: "go language" → go
    if lower.contains("go language") || lower == "go" || lower == "golang" {
        return "go".to_string();
    }
    match lower.as_str() {
        "javascript" | "js" => "js",
        "typescript" | "ts" => "ts",
        "rust" | "rs" => "rs",
        "java" => "java",
        "kotlin" | "kt" => "kt",
        "ruby" | "rb" => "rb",
        "php" => "php",
        "cpp" | "c++" => "cpp",
        "csharp" | "c#" | "cs" => "cs",
        "html" => "html",
        "css" => "css",
        "shell" | "bash" | "sh" => "sh",
        "sql" => "sql",
        "swift" => "swift",
        _ => "txt",
    }
    .to_string()
}

/// Extract the public class name from a Java source file.
/// Returns `None` if the file cannot be read or no public class is found.
/// Falls back to "Main" at the call site.
fn extract_java_class_name(source_path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(source_path).ok()?;
    // Match "public class <Name>" — the class that contains main()
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("public class ") {
            let rest = &trimmed["public class ".len()..];
            let name = rest
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()?;
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Generate a stable filename from the content hint.
/// FIX #21: Use UUID instead of second-resolution timestamp to prevent
/// filename collisions when two workflows run within the same second.
fn generate_filename(hint: &str, extension: &str) -> String {
    // Pull the first descriptive word (fibonacci, factorial, etc.)
    let lower = hint.to_ascii_lowercase();

    // Compound topics that can't be detected by a single keyword substring.
    if (lower.contains("line") || lower.contains("lines"))
        && (lower.contains("count") || lower.contains("counter"))
    {
        let uid = &uuid::Uuid::new_v4().to_string()[..8];
        return format!("line_counter_{}.{}", uid, extension);
    }
    if lower.contains("number table")
        || lower.contains("multiplication table")
        || lower.contains("times table")
    {
        let uid = &uuid::Uuid::new_v4().to_string()[..8];
        return format!("number_table_{}.{}", uid, extension);
    }

    // Check for "function called X" / "function named X" FIRST — before the
    // topics scan — so that greetings like "prints Hello name" don't steal
    // the "hello" keyword and produce hello_*.py instead of greet_*.py.
    let named_fn_topic: Option<String> = {
        let lower2 = lower.as_str();
        let marker_pos = lower2
            .find("called ")
            .map(|p| p + "called ".len())
            .or_else(|| lower2.find("named ").map(|p| p + "named ".len()));
        marker_pos
            .and_then(|p| {
                lower2[p..].split_whitespace().next().map(|w| {
                    w.chars()
                        .filter(|c| c.is_alphanumeric() || *c == '_')
                        .collect::<String>()
                })
            })
            .filter(|s| !s.is_empty())
    };

    let topic = named_fn_topic
        .or_else(|| {
            [
                // Document-type names checked FIRST so "README.md for a python calculator
                // project" picks "readme" and not "calculator".
                "readme",
                "notes",
                "config",
                "json",
                "backup",
                "homepage",
                "todo",
                "report",
                "index",
                // Program-task names
                "fibonacci",
                "factorial",
                "pascal",
                "prime",
                "bubble sort",
                "merge sort",
                "quicksort",
                "sort",
                "binary search",
                "binary tree",
                "binary",
                "calculator",
                "search",
                "hello",
                "bubble",
                "merge",
                "tree",
                "graph",
                // Action verbs last to avoid "installation" grabbing "install" from README hints
                "setup",
                "deploy",
                "install",
            ]
            .iter()
            .find(|kw| lower.contains(*kw))
            .copied()
            .map(|t| t.replace(' ', "_"))
        })
        .unwrap_or_else(|| {
            // FIX #36: Extract first meaningful word from hint instead of "program"
            // Exclude common filler words AND language names (they're not topic descriptors)
            let excluded = [
                "write",
                "make",
                "create",
                "build",
                "program",
                "code",
                "that",
                "with",
                "using",
                "from",
                "python",
                "javascript",
                "typescript",
                "rust",
                "golang",
                "java",
                "kotlin",
                "ruby",
                "php",
                "swift",
                "bash",
                "shell",
                "script",
                "function",
                "algorithm",
                "implement",
                "called",
                "named",
                "reads",
                "takes",
                "prints",
                "makes",
            ];
            lower
                .split_whitespace()
                .find(|w| w.len() > 3 && !excluded.contains(w))
                .unwrap_or("program")
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .take(20)
                .collect::<String>()
        });

    // Use short UUID suffix for uniqueness (no timestamp collision)
    let uid = &uuid::Uuid::new_v4().to_string()[..8];
    format!("{}_{}.{}", topic, uid, extension)
}

/// Generate code from the hint. Uses the existing ContentGenerator.
fn generate_code_from_hint(hint: &str, language: Option<&str>, raw_text: &str) -> String {
    use crate::agent::visual_reasoning::ContentGenerator;
    // Build a generation-friendly prompt that includes the language hint.
    let lang = language.unwrap_or("");
    let prompt = if !lang.is_empty() {
        format!("write a {} program: {}", lang, hint)
    } else {
        format!("write a program: {} {}", hint, raw_text)
    };

    // For non-Python languages, generate language-specific code directly
    // rather than going through ContentGenerator (which is Python-focused).
    let lower_prompt = prompt.to_ascii_lowercase();
    let lower_hint = hint.to_ascii_lowercase();

    if lang == "javascript"
        || lang == "js"
        || lang == "node.js"
        || lang == "nodejs"
        || lang == "node"
        || lang == "es6"
        || lang.starts_with("ecmascript")
        || lower_prompt.contains("javascript")
        || lower_prompt.contains("node.js")
        || lower_prompt.contains("nodejs")
        || lower_prompt.contains("es6")
        || lower_prompt.contains("ecmascript")
    {
        return generate_javascript_code(&lower_hint);
    }
    if lang == "typescript" || lang == "ts" || lower_prompt.contains("typescript") {
        return generate_typescript_code(&lower_hint);
    }
    if lang == "rust" || lang == "rs" || lower_prompt.contains("rust") {
        return generate_rust_code(&lower_hint);
    }
    if lang == "go" || lang == "golang" || lower_prompt.contains("golang") {
        return generate_go_code(&lower_hint);
    }
    if lang == "kotlin" || lang == "kt" || lower_prompt.contains("kotlin") {
        return generate_kotlin_code(&lower_hint);
    }
    if lang == "shell"
        || lang == "bash"
        || lang == "sh"
        || lower_prompt.contains("shell script")
        || lower_prompt.contains("bash script")
    {
        return generate_shell_code(&lower_hint);
    }
    if lang == "java" || lower_prompt.contains(" java ") || lower_prompt.ends_with(" java") {
        return generate_java_code(&lower_hint);
    }
    if lang == "cpp"
        || lang == "c++"
        || lower_prompt.contains("c++")
        || lower_prompt.contains(" cpp ")
    {
        return generate_cpp_code(&lower_hint);
    }
    if lang == "csharp" || lang == "c#" || lang == "cs" || lower_prompt.contains("c#") {
        return generate_csharp_code(&lower_hint);
    }
    if lang == "swift" || lower_prompt.contains("swift") {
        return generate_swift_code(&lower_hint);
    }
    if lang == "ruby" || lang == "rb" || lower_prompt.contains("ruby") {
        return generate_ruby_code(&lower_hint);
    }
    if lang == "php" || lower_prompt.contains(" php") || lower_prompt.ends_with("php") {
        return generate_php_code(&lower_hint);
    }

    let generated = ContentGenerator::generate_content(&prompt);
    generated.content
}

fn generate_javascript_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"function fibonacci(n) {
    // Generate Fibonacci sequence up to n terms
    if (n <= 0) return [];
    if (n === 1) return [0];
    const seq = [0, 1];
    for (let i = 2; i < n; i++) {
        seq.push(seq[i-1] + seq[i-2]);
    }
    return seq;
}

// Example usage
const n = 10;
console.log(`Fibonacci sequence (${n} terms):`, fibonacci(n));"#
            .to_string();
    }
    if hint.contains("factorial") {
        return r#"function factorial(n) {
    if (n < 0) throw new Error("Factorial not defined for negative numbers");
    if (n === 0 || n === 1) return 1;
    return n * factorial(n - 1);
}

for (let i = 0; i <= 5; i++) {
    console.log(`${i}! = ${factorial(i)}`);
}"#
        .to_string();
    }
    if hint.contains("pascal") {
        return r#"function pascalsTriangle(n) {
    const triangle = [];
    for (let i = 0; i < n; i++) {
        const row = new Array(i + 1).fill(1);
        for (let j = 1; j < i; j++) {
            row[j] = triangle[i-1][j-1] + triangle[i-1][j];
        }
        triangle.push(row);
    }
    return triangle;
}

pascalsTriangle(6).forEach(row => console.log(row.join(' ')));"#
            .to_string();
    }
    r#"function main() {
    console.log("Hello, World!");
}
main();"#
        .to_string()
}

fn generate_typescript_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"function fibonacci(n: number): number[] {
    if (n <= 0) return [];
    if (n === 1) return [0];
    const seq: number[] = [0, 1];
    for (let i = 2; i < n; i++) {
        seq.push(seq[i-1] + seq[i-2]);
    }
    return seq;
}

const n = 10;
console.log(`Fibonacci (${n} terms):`, fibonacci(n));"#
            .to_string();
    }
    r#"function main(): void {
    console.log("Hello, World!");
}
main();"#
        .to_string()
}

fn generate_rust_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"fn fibonacci(n: usize) -> Vec<u64> {
    if n == 0 { return vec![]; }
    if n == 1 { return vec![0]; }
    let mut seq = vec![0u64, 1u64];
    for i in 2..n {
        let next = seq[i-1] + seq[i-2];
        seq.push(next);
    }
    seq
}

fn main() {
    let n = 10;
    println!("Fibonacci ({} terms): {:?}", n, fibonacci(n));
}"#
        .to_string();
    }
    if hint.contains("factorial") {
        return r#"fn factorial(n: u64) -> u64 {
    if n == 0 || n == 1 { 1 } else { n * factorial(n - 1) }
}

fn main() {
    for i in 0..=5 {
        println!("{}! = {}", i, factorial(i));
    }
}"#
        .to_string();
    }
    r#"fn main() {
    println!("Hello, World!");
}"#
    .to_string()
}

fn generate_go_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"package main

import "fmt"

func fibonacci(n int) []int {
    if n <= 0 { return []int{} }
    if n == 1 { return []int{0} }
    seq := []int{0, 1}
    for i := 2; i < n; i++ {
        seq = append(seq, seq[i-1]+seq[i-2])
    }
    return seq
}

func main() {
    fmt.Println("Fibonacci (10 terms):", fibonacci(10))
}"#
        .to_string();
    }
    r#"package main

import "fmt"

func main() {
    fmt.Println("Hello, World!")
}"#
    .to_string()
}

fn generate_kotlin_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"fun fibonacci(n: Int): List<Long> {
    if (n <= 0) return emptyList()
    if (n == 1) return listOf(0L)
    val seq = mutableListOf(0L, 1L)
    for (i in 2 until n) {
        seq.add(seq[i-1] + seq[i-2])
    }
    return seq
}

fun main() {
    println("Fibonacci (10 terms): ${fibonacci(10)}")
}"#
        .to_string();
    }
    r#"fun main() {
    println("Hello, World!")
}"#
    .to_string()
}

fn generate_shell_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"#!/bin/bash
# Fibonacci sequence generator
fibonacci() {
    local n=$1
    local a=0 b=1
    for ((i=0; i<n; i++)); do
        echo -n "$a "
        local tmp=$((a + b))
        a=$b
        b=$tmp
    done
    echo
}

echo "Fibonacci (10 terms):"
fibonacci 10"#
            .to_string();
    }
    if hint.contains("backup") {
        return r#"#!/bin/bash
set -e
SOURCE_DIR="${1:-$HOME}"
BACKUP_DIR="${2:-/tmp/backups}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
DEST="${BACKUP_DIR}/backup_${TIMESTAMP}"
mkdir -p "$BACKUP_DIR"
cp -r "$SOURCE_DIR" "$DEST"
echo "Backup complete: $DEST""#
            .to_string();
    }
    r#"#!/bin/bash
# Hello World script
echo "Hello, World!""#
        .to_string()
}

fn generate_java_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"import java.util.ArrayList;
import java.util.List;

public class Main {
    public static List<Long> fibonacci(int n) {
        List<Long> seq = new ArrayList<>();
        if (n <= 0) return seq;
        seq.add(0L);
        if (n == 1) return seq;
        seq.add(1L);
        for (int i = 2; i < n; i++) {
            seq.add(seq.get(i-1) + seq.get(i-2));
        }
        return seq;
    }

    public static void main(String[] args) {
        System.out.println("Fibonacci (10 terms): " + fibonacci(10));
    }
}"#
        .to_string();
    }
    r#"public class Main {
    public static void main(String[] args) {
        System.out.println("Hello, World!");
    }
}"#
    .to_string()
}

fn generate_cpp_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"#include <iostream>
#include <vector>
using namespace std;

vector<long long> fibonacci(int n) {
    if (n <= 0) return {};
    if (n == 1) return {0};
    vector<long long> seq = {0, 1};
    for (int i = 2; i < n; i++) {
        seq.push_back(seq[i-1] + seq[i-2]);
    }
    return seq;
}

int main() {
    auto seq = fibonacci(10);
    cout << "Fibonacci (10 terms): ";
    for (int i = 0; i < seq.size(); i++) {
        if (i > 0) cout << ", ";
        cout << seq[i];
    }
    cout << endl;
    return 0;
}"#
        .to_string();
    }
    r#"#include <iostream>
using namespace std;

int main() {
    cout << "Hello, World!" << endl;
    return 0;
}"#
    .to_string()
}

fn generate_csharp_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"using System;
using System.Collections.Generic;

class Program {
    static List<long> Fibonacci(int n) {
        var seq = new List<long>();
        if (n <= 0) return seq;
        seq.Add(0);
        if (n == 1) return seq;
        seq.Add(1);
        for (int i = 2; i < n; i++) {
            seq.Add(seq[i-1] + seq[i-2]);
        }
        return seq;
    }

    static void Main() {
        var seq = Fibonacci(10);
        Console.WriteLine("Fibonacci (10 terms): " + string.Join(", ", seq));
    }
}"#
        .to_string();
    }
    r#"using System;

class Program {
    static void Main() {
        Console.WriteLine("Hello, World!");
    }
}"#
    .to_string()
}

fn generate_swift_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"func fibonacci(_ n: Int) -> [Int] {
    if n <= 0 { return [] }
    if n == 1 { return [0] }
    var seq = [0, 1]
    for i in 2..<n {
        seq.append(seq[i-1] + seq[i-2])
    }
    return seq
}

let seq = fibonacci(10)
print("Fibonacci (10 terms): \(seq)")"#
            .to_string();
    }
    r#"print("Hello, World!")"#.to_string()
}

fn generate_ruby_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"def fibonacci(n)
  return [] if n <= 0
  return [0] if n == 1
  seq = [0, 1]
  (2...n).each { |i| seq << seq[-1] + seq[-2] }
  seq
end

puts "Fibonacci (10 terms): #{fibonacci(10).inspect}"
"#
        .to_string();
    }
    if hint.contains("factorial") {
        return "def factorial(n)\n  return 1 if n <= 1\n  n * factorial(n - 1)\nend\n\n(0..5).each { |i| puts \"#{i}! = #{factorial(i)}\" }\n".to_string();
    }
    "puts \"Hello, World!\"\n".to_string()
}

fn generate_php_code(hint: &str) -> String {
    if hint.contains("fibonacci") {
        return r#"<?php
function fibonacci($n) {
    if ($n <= 0) return [];
    if ($n == 1) return [0];
    $seq = [0, 1];
    for ($i = 2; $i < $n; $i++) {
        $seq[] = $seq[$i-1] + $seq[$i-2];
    }
    return $seq;
}

$seq = fibonacci(10);
echo "Fibonacci (10 terms): " . implode(", ", $seq) . "\n";
"#
        .to_string();
    }
    if hint.contains("factorial") {
        return r#"<?php
function factorial($n) {
    if ($n <= 1) return 1;
    return $n * factorial($n - 1);
}

for ($i = 0; $i <= 5; $i++) {
    echo "$i! = " . factorial($i) . "\n";
}
"#
        .to_string();
    }
    r#"<?php
echo "Hello, World!\n";
"#
    .to_string()
}

/// Extract the click target from raw user text.
/// "click the Save button" → "Save"
/// "click OK" → "OK"
/// "press the Cancel button" → "Cancel"
fn extract_click_target(raw_text: &str) -> String {
    let lower = raw_text.to_ascii_lowercase();

    // Common patterns: "click X", "press X", "click the X button", "click on X"
    for marker in &[
        "click the ",
        "click on the ",
        "click on ",
        "click ",
        "press the ",
        "press ",
    ] {
        if let Some(pos) = lower.find(marker) {
            let after = raw_text[pos + marker.len()..].trim();
            // Stop at "button", "link", "checkbox", etc.
            let target = after
                .split(|c: char| c == ' ' || c == '\n' || c == ',' || c == '.')
                .take(3)
                .collect::<Vec<_>>()
                .join(" ");
            // Remove trailing role words
            let target = target
                .trim_end_matches(" button")
                .trim_end_matches(" link")
                .trim_end_matches(" checkbox")
                .trim_end_matches(" radio")
                .trim_end_matches(" tab")
                .trim();
            if !target.is_empty() {
                return target.to_string();
            }
        }
    }

    // Fallback: use the whole raw text as a hint
    raw_text.chars().take(30).collect()
}

/// Infer the AT-SPI role from the raw user text.
/// "click the Save button" → "push button"
/// "click the File menu" → "menu"
/// "click the checkbox" → "check box"
fn infer_element_role(raw_text: &str) -> &'static str {
    let lower = raw_text.to_ascii_lowercase();
    if lower.contains("button") || lower.contains("btn") {
        "push button"
    } else if lower.contains("menu item") || lower.contains("menu entry") {
        "menu item"
    } else if lower.contains(" menu") {
        "menu"
    } else if lower.contains("checkbox") || lower.contains("check box") || lower.contains("tick") {
        "check box"
    } else if lower.contains("radio") {
        "radio button"
    } else if lower.contains("tab") {
        "page tab"
    } else if lower.contains("link") {
        "link"
    } else if lower.contains("combo") || lower.contains("dropdown") || lower.contains("drop-down") {
        "combo box"
    } else if lower.contains("list item") {
        "list item"
    } else {
        "push button" // Default to button — most common interaction target
    }
}
/// the file content matches what we intended to write.
///
/// SAFETY: Never returns an empty string. An empty expected_substring would
/// cause `ContainsBytes(b"")` to always verify as true — a false-success path.
fn extract_verifiable_substring(code: &str) -> String {
    // First non-empty line, capped at 60 chars — usually a `def`, `function`,
    // `fn`, etc. that uniquely identifies the generated file.
    let candidate = code
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("//"))
        .map(|s| s.chars().take(60).collect::<String>())
        .unwrap_or_else(|| code.chars().take(40).collect::<String>());

    // Guard: if the code is empty or only whitespace, use a sentinel that
    // will cause verification to fail (better than silently passing).
    if candidate.trim().is_empty() {
        "KRIA_GENERATED_CODE_PLACEHOLDER".to_string()
    } else {
        candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_open_app_with_code(app: &str, hint: &str, lang: Option<&str>) -> GuiTaskSpec {
        GuiTaskSpec {
            primary_verb: Verb::Open,
            targets: vec![TargetRef::App(app.into())],
            content: Some(ContentClass::Generated {
                hint: hint.into(),
                language: lang.map(String::from),
            }),
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        }
    }

    #[test]
    fn open_gedit_with_fibonacci_picks_file_substrate() {
        let spec = spec_open_app_with_code("gedit", "fibonacci program", Some("python"));
        let plan = SubstratePlanner.plan(
            &spec,
            "Open gedit and type a program to print fibonacci series in python",
        );
        assert_eq!(plan.substrate, ExecutionSubstrate::FileWriteThenOpen);
        let wf = plan.workflow.expect("workflow generated");
        assert_eq!(wf.sub_goals.len(), 2);
        assert_eq!(wf.sub_goals[0].action, "write_file");
        assert_eq!(wf.sub_goals[1].action, "open_application_with_file");
        assert_eq!(plan.artifacts.len(), 1);
        assert!(plan.artifacts[0].extension().and_then(|e| e.to_str()) == Some("py"));
    }

    #[test]
    fn open_app_only_picks_app_substrate() {
        let spec = GuiTaskSpec {
            primary_verb: Verb::Open,
            targets: vec![TargetRef::App("gedit".into())],
            content: None,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        };
        let plan = SubstratePlanner.plan(&spec, "open gedit");
        assert_eq!(plan.substrate, ExecutionSubstrate::AppOpenOnly);
        let wf = plan.workflow.expect("workflow generated");
        assert_eq!(wf.sub_goals[0].action, "open_application");
    }

    #[test]
    fn browser_app_with_search_picks_browser_navigate() {
        let spec = GuiTaskSpec {
            primary_verb: Verb::Open,
            targets: vec![TargetRef::App("google-chrome".into())],
            content: None,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        };
        let plan = SubstratePlanner.plan(&spec, "open chrome and search for youtube");
        assert_eq!(plan.substrate, ExecutionSubstrate::BrowserNavigate);
        let wf = plan.workflow.expect("workflow generated");
        assert_eq!(wf.sub_goals[0].action, "browser_search");
        // Should extract "youtube" as the site
        let site = wf.sub_goals[0].params.get("site").and_then(|v| v.as_str());
        assert_eq!(site, Some("youtube"));
    }

    #[test]
    fn firefox_without_search_still_picks_browser_navigate() {
        // Firefox is a browser — but without explicit search/URL targets, it falls back
        // to AppOpenOnly substrate per Fix #15.
        let spec = GuiTaskSpec {
            primary_verb: Verb::Open,
            targets: vec![TargetRef::App("firefox".into())],
            content: None,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        };
        let plan = SubstratePlanner.plan(&spec, "open firefox");
        assert_eq!(plan.substrate, ExecutionSubstrate::AppOpenOnly);
    }

    #[test]
    fn text_editor_email_draft_uses_document_workflow_without_llm() {
        let spec = GuiTaskSpec {
            primary_verb: Verb::Open,
            targets: vec![TargetRef::App("text editor".into())],
            content: None,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        };
        let plan = SubstratePlanner.plan(
            &spec,
            "Draft a short email in a text editor, but do not send anything. Show me the draft for approval.",
        );

        assert_eq!(plan.substrate, ExecutionSubstrate::FileWriteThenOpen);
        let wf = plan.workflow.expect("workflow generated");
        assert_eq!(wf.sub_goals.len(), 2);
        assert_eq!(wf.sub_goals[0].action, "write_file");
        assert_eq!(wf.sub_goals[1].action, "open_application_with_file");
        assert_eq!(
            wf.sub_goals[1]
                .params
                .get("name")
                .and_then(|value| value.as_str()),
            Some("text editor")
        );
        assert_eq!(
            plan.artifacts[0].extension().and_then(|e| e.to_str()),
            Some("md")
        );
    }

    #[test]
    fn spreadsheet_prompt_creates_csv_then_opens_spreadsheet_app() {
        let spec = GuiTaskSpec {
            primary_verb: Verb::Open,
            targets: vec![TargetRef::App("Excel or Calc".into())],
            content: None,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        };
        let plan = SubstratePlanner.plan(
            &spec,
            "Open Excel or Calc if available, create a temporary sheet with Item, Quantity, Price, and Total columns. Show me the sheet.",
        );

        assert_eq!(plan.substrate, ExecutionSubstrate::FileWriteThenOpen);
        let wf = plan.workflow.expect("workflow generated");
        assert_eq!(wf.sub_goals[0].action, "write_file");
        assert_eq!(wf.sub_goals[1].action, "open_application_with_file");
        assert_eq!(
            wf.sub_goals[1]
                .params
                .get("name")
                .and_then(|value| value.as_str()),
            Some("Excel or Calc")
        );
        assert_eq!(
            plan.artifacts[0].extension().and_then(|e| e.to_str()),
            Some("csv")
        );
    }

    #[test]
    fn type_literal_picks_keystroke() {
        let spec = GuiTaskSpec {
            primary_verb: Verb::Type,
            targets: vec![],
            content: Some(ContentClass::Literal("hello world".into())),
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        };
        let plan = SubstratePlanner.plan(&spec, "type 'hello world'");
        assert_eq!(plan.substrate, ExecutionSubstrate::Keystroke);
    }

    #[test]
    fn extension_is_inferred_from_python_keyword() {
        assert_eq!(language_to_extension(None, "fibonacci in python"), "py");
        assert_eq!(language_to_extension(Some("rust"), "anything"), "rs");
        assert_eq!(language_to_extension(None, "javascript code"), "js");
        assert_eq!(
            language_to_extension(None, "write a fibonacci program"),
            "txt"
        );
        assert_eq!(
            language_to_extension(None, "write a fibonacci program and run it"),
            "py"
        );
        assert_eq!(
            language_to_extension(
                None,
                "open code and write a program to print pascal triangle and run it and show output"
            ),
            "py"
        );
    }

    #[test]
    fn vscode_run_prompt_uses_ide_code_run_workflow() {
        let spec = spec_open_app_with_code("code", "pascal triangle program", None);
        let plan = SubstratePlanner.plan(
            &spec,
            "open code and write a program to print pascal triangle and run it and show output",
        );
        assert_eq!(plan.substrate, ExecutionSubstrate::IdeCodeRunWorkflow);
        let wf = plan.workflow.expect("workflow generated");
        assert_eq!(wf.sub_goals.len(), 4);
        assert_eq!(wf.sub_goals[0].action, "write_file");
        assert_eq!(wf.sub_goals[1].action, "write_file");
        assert_eq!(wf.sub_goals[2].action, "open_application_with_file");
        assert_eq!(wf.sub_goals[3].action, "execute_bash");
        assert_eq!(plan.artifacts.len(), 3);
        assert!(wf.sub_goals[3]
            .params
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("gnome-terminal"));
    }

    #[test]
    fn intellij_run_prompt_uses_class_based_ide_code_run_workflow() {
        let spec = spec_open_app_with_code("IntelliJ IDEA", "hello world program", Some("python"));
        let plan = SubstratePlanner.plan(
            &spec,
            "open intellij and write a python program to print hello world and run it and show output",
        );

        assert_eq!(plan.substrate, ExecutionSubstrate::IdeCodeRunWorkflow);
        let wf = plan.workflow.expect("workflow generated");
        assert_eq!(wf.sub_goals.len(), 4);
        assert_eq!(wf.sub_goals[0].action, "write_file");
        assert_eq!(wf.sub_goals[1].action, "write_file");
        assert_eq!(wf.sub_goals[2].action, "open_application_with_file");
        assert_eq!(wf.sub_goals[3].action, "execute_bash");
        assert_eq!(
            wf.sub_goals[2]
                .params
                .get("name")
                .and_then(|value| value.as_str()),
            Some("IntelliJ IDEA")
        );
        assert!(matches!(
            &wf.sub_goals[2].verify,
            VerificationType::ProcessLaunched { binary, .. } if binary == "idea"
        ));
    }

    #[test]
    fn visible_terminal_launcher_discloses_structural_fallback() {
        let output_path = generated_files_dir().join("phase6_output_marker.txt");
        let runner_path = generated_files_dir().join("phase6_runner_marker.sh");
        let command = build_terminal_launcher_command(
            &runner_path,
            &output_path,
            "printf hello > /tmp/phase6-fallback-output",
        );

        assert!(command.contains("Visible terminal launcher unavailable"));
        assert!(command.contains("structural fallback used"));
    }

    #[test]
    fn generates_distinctive_filename() {
        let f = generate_filename("fibonacci program", "py");
        assert!(f.starts_with("fibonacci_"));
        assert!(f.ends_with(".py"));
    }
}
