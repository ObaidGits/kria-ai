//! Browser Workflow Eval Suite.
//!
//! Validates that KRIA correctly navigates browsers, extracts content,
//! and surfaces results to the user.
//!
//! Validates:
//! - Browser opens correctly
//! - CDP semantics work
//! - DOM reasoning extracts the right content
//! - Operational continuity during login flows
//! - Semantic completion (result surfaced, not just "browser opened")

use crate::workflow_eval::contracts::browser_contract;
use crate::workflow_eval::types::{
    EvalWorkflowCategory, SafetyClass, SemanticCompletionContract, WorkflowEvalCase,
};
use std::time::Duration;

fn browser_case(
    id: &str,
    description: &str,
    prompt: &str,
    contract: SemanticCompletionContract,
    safety: SafetyClass,
    tags: &[&str],
) -> WorkflowEvalCase {
    WorkflowEvalCase {
        id: id.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        category: EvalWorkflowCategory::Browser,
        contract,
        safety_class: safety,
        interruption: None,
        timeout: Duration::from_secs(90),
        requires_daemon: true,
        requires_display: true,
        tags: {
            let mut t = tags.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            t.push("browser".into());
            t
        },
        eval_notes: format!(
            "Validates browser cognition. Case: {}. \
             FAIL if browser opened silently without surfacing the content.",
            id
        ),
    }
}

fn browser_with_signal(signal: &str) -> SemanticCompletionContract {
    let mut c = browser_contract();
    c.semantic_success_signals = vec![signal.to_string()];
    c.required_observable_outputs[0].response_must_contain = vec![signal.to_string()];
    c
}

pub fn browser_suite() -> Vec<WorkflowEvalCase> {
    vec![
        // ── Search and extract ────────────────────────────────────────────────
        browser_case(
            "wf-browser-001-weather-search",
            "Search weather and summarize result visibly",
            "search for the weather in london and show me the result",
            {
                let mut c = browser_contract();
                c.success_definition = "Weather search result summarized and shown to user".into();
                c.semantic_success_signals = vec![
                    "weather".into(),
                    "temperature".into(),
                    "°c".into(),
                    "°f".into(),
                    "degrees".into(),
                ];
                c
            },
            SafetyClass::Safe,
            &["weather", "search", "extract"],
        ),
        browser_case(
            "wf-browser-002-youtube-search",
            "Open YouTube and search for a video — must show video titles",
            "open youtube and search for lo-fi music and show me the first few results",
            browser_with_signal("video"),
            SafetyClass::Safe,
            &["youtube", "search", "media"],
        ),
        browser_case(
            "wf-browser-003-wikipedia-summary",
            "Open Wikipedia page and summarize content",
            "open wikipedia and find the article on the Eiffel Tower and give me a summary",
            browser_with_signal("eiffel"),
            SafetyClass::Safe,
            &["wikipedia", "summarize", "extract"],
        ),
        // ── Navigation and interaction ────────────────────────────────────────
        browser_case(
            "wf-browser-004-github-repo",
            "Navigate to a GitHub repo and extract its description",
            "open the github repository for python and tell me what it says in the description",
            browser_with_signal("python"),
            SafetyClass::Safe,
            &["github", "navigate", "extract"],
        ),
        browser_case(
            "wf-browser-005-news-headlines",
            "Search news and surface headlines",
            "open the browser and find the latest tech news headlines and show them to me",
            {
                let mut c = browser_contract();
                c.semantic_success_signals = vec!["news".into(), "headline".into()];
                c
            },
            SafetyClass::Safe,
            &["news", "headlines", "current-events"],
        ),
        // ── Download and open ─────────────────────────────────────────────────
        browser_case(
            "wf-browser-006-download-and-open",
            "Download a text file from the web and open it",
            "download the readme from a simple github repo and open it in a text editor",
            {
                let mut c = browser_contract();
                c.success_definition = "File downloaded and opened in text editor".into();
                c.semantic_success_signals = vec!["downloaded".into(), "opened".into()];
                c
            },
            SafetyClass::Reversible,
            &["download", "open", "file"],
        ),
        // ── Interruption: login flow ──────────────────────────────────────────
        browser_case(
            "wf-browser-007-login-interruption",
            "Handle login form appearance during browsing workflow",
            "open the browser and go to gmail — if a login page appears, tell me",
            {
                let mut c = browser_contract();
                c.success_definition =
                    "KRIA detects login page and informs user rather than proceeding blindly"
                        .into();
                c.semantic_success_signals = vec![
                    "login".into(),
                    "sign in".into(),
                    "authentication".into(),
                    "credentials".into(),
                ];
                c.forbidden_silent_completion_patterns =
                    vec!["task completed".into(), "browser is open".into()];
                c
            },
            SafetyClass::Safe,
            &["login", "interruption", "auth-detection"],
        ),
    ]
}
