//! RuleIntentCompiler — pattern-matching semantic intent normalizer.
//!
//! # Design Contract
//!
//! Replaces `NoopIntentCompiler` as the **default production compiler**.
//! Uses deterministic regex/keyword heuristics to produce a typed `GuiTaskSpec`
//! from natural-language user text. No LLM calls, no external I/O, no side effects.
//!
//! # Coverage
//!
//! | Pattern                    | Verb     | TargetRef              |
//! |----------------------------|----------|------------------------|
//! | "open [app]"               | Open     | App(name)              |
//! | "open [file] in / with"    | Open     | File(path)             |
//! | "navigate / go to [url]"   | Open     | Url(url)               |
//! | "type / write [text]"      | Type     | Element(focused)       |
//! | "click [element]"          | Click    | Element(label)         |
//! | "run / execute [cmd]"      | Run      | App(cmd)               |
//! | "save / save as"           | Save     | (none)                 |
//! | "close [app]"              | Close    | App(name)              |
//! | "switch to [app]"          | Switch   | App(name)              |
//! | (other)                    | Other    | (none)                 |
//!
//! # Ambiguity
//!
//! The compiler raises `Ambiguity::AppNotSpecified` for `Open` without a
//! clear target, and `Ambiguity::ContentScopeUnclear` for `Type` without
//! quoted/parenthesised content.

use crate::agent::intent_compiler::{
    Ambiguity, ClarifyRequest, ContentClass, GuiTaskSpec, IntentCompiler, TargetRef, Verb,
};
use crate::agent::turn_gate::IntentEnvelope;

// ─── URL detection ────────────────────────────────────────────────────────────

/// Determine if a token is genuinely a URL, not a sentence-ending word with punctuation.
///
/// Previous implementation used `s.contains('.') && s.len() > 4` which caused
/// catastrophic false positives: "output.", "results.", "loaded." were all
/// classified as URLs, causing KRIA to open "https://output./" in the browser.
///
/// This implementation requires STRUCTURAL URL evidence:
/// - Explicit scheme (http://, https://)
/// - www. prefix
/// - A valid TLD pattern (word.tld where tld is 2-6 alpha chars, not sentence punctuation)
fn looks_like_url(s: &str) -> bool {
    // Strip trailing punctuation that's clearly sentence-ending
    let cleaned = s.trim_end_matches(|c: char| c == '.' || c == ',' || c == ';' || c == '!' || c == '?' || c == ')' || c == ']');

    // Explicit scheme — always a URL
    if cleaned.starts_with("http://") || cleaned.starts_with("https://") {
        return true;
    }

    // www. prefix — always a URL
    if cleaned.starts_with("www.") && cleaned.len() > 5 {
        return true;
    }

    // Structural TLD check: must have at least one dot with a valid TLD-like suffix
    // e.g., "example.com", "docs.rs", "github.io"
    // Rejects: "output", "results", "Mr.", "e.g.", "i.e."
    // Also handles URLs with paths: "httpbin.org/get"
    let domain_part = if let Some(slash_pos) = cleaned.find('/') {
        &cleaned[..slash_pos]
    } else {
        cleaned
    };

    if let Some(dot_pos) = domain_part.rfind('.') {
        let before_dot = &domain_part[..dot_pos];
        let after_dot = &domain_part[dot_pos + 1..];

        // TLD must be 2-6 alphabetic characters (com, org, net, io, dev, etc.)
        let valid_tld = after_dot.len() >= 2
            && after_dot.len() <= 6
            && after_dot.chars().all(|c| c.is_ascii_alphabetic());

        // Before the dot must have at least 2 chars and no spaces
        let valid_domain = before_dot.len() >= 2
            && !before_dot.contains(' ')
            && before_dot.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_');

        // Reject common abbreviations and sentence fragments
        let is_abbreviation = matches!(
            cleaned.to_lowercase().as_str(),
            "e.g." | "i.e." | "etc." | "vs." | "mr." | "mrs." | "dr." | "st."
                | "no." | "vol." | "fig." | "eq." | "approx." | "dept."
        );

        if valid_tld && valid_domain && !is_abbreviation {
            return true;
        }
    }

    false
}

// ─── Token extraction helpers ─────────────────────────────────────────────────

/// Extract a quoted string from text: `"hello world"` or `'hello world'`
fn extract_quoted(text: &str) -> Option<String> {
    for (open, close) in [("\"", "\""), ("'", "'"), ("\u{2018}", "\u{2019}")] {
        if let Some(start) = text.find(open) {
            let inner = &text[start + open.len()..];
            if let Some(end) = inner.find(close) {
                return Some(inner[..end].trim().to_string());
            }
        }
    }
    None
}

/// Extract a file path from text (anything starting with `/` or `~/`).
fn extract_file_path(text: &str) -> Option<std::path::PathBuf> {
    for token in text.split_whitespace() {
        let clean = token.trim_matches(|c| c == '\'' || c == '"');
        if clean.starts_with('/') || clean.starts_with("~/") {
            return Some(std::path::PathBuf::from(clean));
        }
    }
    None
}

/// Extract the token after a keyword (e.g., `"open firefox"` → `"firefox"`).
fn token_after(text: &str, keyword: &str) -> Option<String> {
    let lower = text.to_lowercase();
    if let Some(idx) = lower.find(keyword) {
        let rest = text[idx + keyword.len()..].trim();
        let token = rest
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.');
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

/// Extract the rest of the string after a keyword (for multi-word targets).
fn rest_after(text: &str, keyword: &str) -> Option<String> {
    let lower = text.to_lowercase();
    if let Some(idx) = lower.find(keyword) {
        let rest = text[idx + keyword.len()..].trim();
        if !rest.is_empty() {
            return Some(rest.to_string());
        }
    }
    None
}

// ─── Compiler ─────────────────────────────────────────────────────────────────

/// Deterministic rule-based intent compiler.
///
/// Replaces `NoopIntentCompiler` in production. Falls back gracefully to
/// `Verb::Other` with no ambiguities when no pattern matches.
pub struct RuleIntentCompiler;

#[async_trait::async_trait]
impl IntentCompiler for RuleIntentCompiler {
    async fn compile(
        &self,
        user_text: &str,
        _intent: &IntentEnvelope,
    ) -> Result<GuiTaskSpec, ClarifyRequest> {
        let text = user_text.trim();
        let lower = text.to_lowercase();

        // ── Open ─────────────────────────────────────────────────────────────
        let open_exact = lower == "open" || lower == "launch" || lower == "start";
        if open_exact
            || lower.starts_with("open ")
            || lower.starts_with("launch ")
            || lower.starts_with("start ")
        {
            if open_exact {
                return Err(ClarifyRequest {
                    question: "What would you like to open?".to_string(),
                    options: vec![
                        "An application".to_string(),
                        "A file".to_string(),
                        "A URL".to_string(),
                    ],
                });
            }
            let kw = if lower.starts_with("open ") {
                "open "
            } else if lower.starts_with("launch ") {
                "launch "
            } else {
                "start "
            };

            let rest = text[kw.len()..].trim();
            let rest_lower = rest.to_lowercase();

            // "open /path/to/file" or "open ~/something"
            if let Some(path) = extract_file_path(rest) {
                return Ok(GuiTaskSpec {
                    primary_verb: Verb::Open,
                    targets: vec![TargetRef::File(path)],
                    content: None,
                    declared_preconditions: vec![],
                    declared_success_criteria: vec![],
                    ambiguities: vec![],
                });
            }

            // "open https://..." or "open www.example.com"
            if looks_like_url(rest) {
                return Ok(GuiTaskSpec {
                    primary_verb: Verb::Open,
                    targets: vec![TargetRef::Url(rest.to_string())],
                    content: None,
                    declared_preconditions: vec![],
                    declared_success_criteria: vec![],
                    ambiguities: vec![],
                });
            }

            // "open file.txt in gedit" or "open file with nano"
            if rest_lower.contains(" in ") || rest_lower.contains(" with ") {
                let sep = if rest_lower.contains(" in ") {
                    " in "
                } else {
                    " with "
                };
                let parts: Vec<&str> = rest.splitn(2, sep).collect();
                if parts.len() == 2 {
                    let file_part = parts[0].trim();
                    let app_part = parts[1].trim();
                    return Ok(GuiTaskSpec {
                        primary_verb: Verb::Open,
                        targets: vec![
                            TargetRef::File(std::path::PathBuf::from(file_part)),
                            TargetRef::App(app_part.to_string()),
                        ],
                        content: None,
                        declared_preconditions: vec![],
                        declared_success_criteria: vec![],
                        ambiguities: vec![],
                    });
                }
            }

            // "open firefox" — app name
            if !rest.is_empty() {
                return Ok(GuiTaskSpec {
                    primary_verb: Verb::Open,
                    targets: vec![TargetRef::App(rest.to_string())],
                    content: None,
                    declared_preconditions: vec![],
                    declared_success_criteria: vec![],
                    ambiguities: vec![],
                });
            }

            // Fallthrough: Verb::Other if we exhausted all patterns
            return Ok(GuiTaskSpec {
                primary_verb: Verb::Other("open_unknown".to_string()),
                targets: vec![],
                content: None,
                declared_preconditions: vec![],
                declared_success_criteria: vec![],
                ambiguities: vec![Ambiguity::AppNotSpecified],
            });
        }

        // ── Navigate / Go to ─────────────────────────────────────────────────
        if lower.starts_with("navigate to ")
            || lower.starts_with("go to ")
            || lower.starts_with("visit ")
        {
            let kw = if lower.starts_with("navigate to ") {
                "navigate to "
            } else if lower.starts_with("go to ") {
                "go to "
            } else {
                "visit "
            };
            let url_str = text[kw.len()..].trim();
            let url = if looks_like_url(url_str) {
                url_str.to_string()
            } else {
                format!("https://{}", url_str)
            };
            return Ok(GuiTaskSpec {
                primary_verb: Verb::Open,
                targets: vec![TargetRef::Url(url)],
                content: None,
                declared_preconditions: vec![],
                declared_success_criteria: vec![],
                ambiguities: vec![],
            });
        }

        // ── "Write a [language] script at [path]" — deterministic file-write pattern ──
        // Recognizes:
        //   "Write a Python script at /tmp/foo.py that prints hello"
        //   "Create a Rust program at /home/user/main.rs that ..."
        //   "Generate a bash script at /tmp/x.sh that ..."
        // This produces a Verb::Open with both an App target (the language interpreter)
        // and a File target (the path), allowing the substrate planner to recognize it
        // as a file-write workflow without needing the LLM.
        {
            let lower_text = lower.as_str();
            let script_pattern_starts = [
                "write a python", "write a rust", "write a javascript", "write a js ",
                "write a bash", "write a shell", "write a node", "write a ruby",
                "write a go ", "write a c ", "write a c++", "write a cpp",
                "create a python", "create a rust", "create a javascript", "create a js ",
                "create a bash", "create a shell", "create a node", "create a ruby",
                "generate a python", "generate a rust", "generate a bash",
            ];
            let matched_prefix = script_pattern_starts.iter().find(|p| lower_text.starts_with(*p));
            if let Some(prefix) = matched_prefix {
                // Extract the language from the prefix
                let language = if prefix.contains("python") { "python" }
                    else if prefix.contains("rust") { "rust" }
                    else if prefix.contains("javascript") || prefix.contains(" js ") { "javascript" }
                    else if prefix.contains("bash") || prefix.contains("shell") { "bash" }
                    else if prefix.contains("node") { "node" }
                    else if prefix.contains("ruby") { "ruby" }
                    else if prefix.contains(" go ") { "go" }
                    else if prefix.contains("c++") || prefix.contains("cpp") { "cpp" }
                    else if prefix.contains(" c ") { "c" }
                    else { "text" };

                // Try to extract path: "at /path/to/file.ext" or "in /path/to/file.ext"
                let path = extract_file_path(text);

                // Extract the content hint (everything after "that" or after the path)
                let hint = if let Some(idx) = lower_text.find(" that ") {
                    text[idx + 6..].trim().to_string()
                } else if lower_text.contains(" with ") {
                    if let Some(idx) = lower_text.find(" with ") {
                        text[idx + 6..].trim().to_string()
                    } else {
                        text.to_string()
                    }
                } else {
                    text.to_string()
                };

                let mut targets = Vec::new();
                if let Some(p) = path {
                    targets.push(TargetRef::File(p));
                }
                // Add a generic "code" or interpreter target so substrate planner can pick
                targets.push(TargetRef::App(language.to_string()));

                return Ok(GuiTaskSpec {
                    primary_verb: Verb::Open,  // treated as "create + open"
                    targets,
                    content: Some(ContentClass::Generated {
                        hint,
                        language: Some(language.to_string()),
                    }),
                    declared_preconditions: vec![],
                    declared_success_criteria: vec![],
                    ambiguities: vec![],
                });
            }
        }

        // ── Type / Write ──────────────────────────────────────────────────────
        let type_kws = ["type ", "enter ", "input ", "write "];
        for kw in &type_kws {
            if lower.starts_with(kw) {
                let rest = &text[kw.len()..];
                let content = if let Some(quoted) = extract_quoted(rest) {
                    ContentClass::Literal(quoted)
                } else {
                    // Treat the whole rest as text to type
                    ContentClass::Literal(rest.trim().to_string())
                };
                let ambiguities = if matches!(content, ContentClass::Literal(ref s) if s.is_empty())
                {
                    vec![Ambiguity::ContentScopeUnclear]
                } else {
                    vec![]
                };
                return Ok(GuiTaskSpec {
                    primary_verb: Verb::Type,
                    targets: vec![TargetRef::Element("focused".to_string())],
                    content: Some(content),
                    declared_preconditions: vec![],
                    declared_success_criteria: vec![],
                    ambiguities,
                });
            }
        }

        // ── Click ─────────────────────────────────────────────────────────────
        if lower.starts_with("click ") || lower.starts_with("press ") {
            let kw = if lower.starts_with("click ") {
                "click "
            } else {
                "press "
            };
            let element = text[kw.len()..].trim();
            if element.is_empty() {
                return Err(ClarifyRequest {
                    question: "Which element should I click?".to_string(),
                    options: vec![
                        "A button".to_string(),
                        "A link".to_string(),
                        "Other element".to_string(),
                    ],
                });
            }
            return Ok(GuiTaskSpec {
                primary_verb: Verb::Click,
                targets: vec![TargetRef::Element(element.to_string())],
                content: None,
                declared_preconditions: vec![],
                declared_success_criteria: vec![],
                ambiguities: vec![],
            });
        }

        // ── Run / Execute ─────────────────────────────────────────────────────
        if lower.starts_with("run ") || lower.starts_with("execute ") {
            let kw = if lower.starts_with("run ") {
                "run "
            } else {
                "execute "
            };
            let cmd = text[kw.len()..].trim();
            return Ok(GuiTaskSpec {
                primary_verb: Verb::Run,
                targets: vec![TargetRef::App(cmd.to_string())],
                content: None,
                declared_preconditions: vec![],
                declared_success_criteria: vec![],
                ambiguities: vec![],
            });
        }

        // ── Save ──────────────────────────────────────────────────────────────
        if lower == "save" || lower.starts_with("save as") || lower.starts_with("save file") {
            let target = if let Some(path) = extract_file_path(text) {
                vec![TargetRef::File(path)]
            } else {
                vec![]
            };
            return Ok(GuiTaskSpec {
                primary_verb: Verb::Save,
                targets: target,
                content: None,
                declared_preconditions: vec![],
                declared_success_criteria: vec![],
                ambiguities: vec![],
            });
        }

        // ── Close ─────────────────────────────────────────────────────────────
        if lower.starts_with("close ") || lower.starts_with("quit ") || lower.starts_with("exit ") {
            let kw = if lower.starts_with("close ") {
                "close "
            } else if lower.starts_with("quit ") {
                "quit "
            } else {
                "exit "
            };
            let app = text[kw.len()..].trim();
            return Ok(GuiTaskSpec {
                primary_verb: Verb::Close,
                targets: vec![TargetRef::App(app.to_string())],
                content: None,
                declared_preconditions: vec![],
                declared_success_criteria: vec![],
                ambiguities: vec![],
            });
        }

        // ── Switch ────────────────────────────────────────────────────────────
        if lower.starts_with("switch to ") || lower.starts_with("focus ") {
            let kw = if lower.starts_with("switch to ") {
                "switch to "
            } else {
                "focus "
            };
            let app = text[kw.len()..].trim();
            return Ok(GuiTaskSpec {
                primary_verb: Verb::Switch,
                targets: vec![TargetRef::App(app.to_string())],
                content: None,
                declared_preconditions: vec![],
                declared_success_criteria: vec![],
                ambiguities: vec![],
            });
        }

        // ── URL anywhere in text ───────────────────────────────────────────────
        // Only match tokens that are CLEARLY URLs (have scheme or valid domain.tld).
        // The `looks_like_url` function already handles punctuation stripping and
        // structural validation, but we add an extra guard: the token must be the
        // PRIMARY content of the sentence (not just a word ending with a period).
        for token in text.split_whitespace() {
            // Skip tokens that are clearly just words with trailing punctuation
            let cleaned = token.trim_end_matches(|c: char| c == '.' || c == ',' || c == ';' || c == '!' || c == '?');
            if cleaned.is_empty() {
                continue;
            }
            // Only consider it a URL if it passes structural validation
            if looks_like_url(token) {
                // Additional guard: if the token is just a common English word + period,
                // don't treat it as a URL even if looks_like_url passes.
                // This catches edge cases the TLD check might miss.
                let word_lower = cleaned.to_lowercase();
                let is_common_word = matches!(word_lower.as_str(),
                    "output" | "results" | "loaded" | "running" | "available" | "installed"
                    | "complete" | "finished" | "started" | "stopped" | "working" | "ready"
                    | "done" | "failed" | "success" | "error" | "warning" | "info"
                    | "file" | "folder" | "directory" | "process" | "service"
                );
                if is_common_word {
                    continue;
                }
                return Ok(GuiTaskSpec {
                    primary_verb: Verb::Open,
                    targets: vec![TargetRef::Url(cleaned.to_string())],
                    content: None,
                    declared_preconditions: vec![],
                    declared_success_criteria: vec![],
                    ambiguities: vec![],
                });
            }
        }

        // ── Fallback: Verb::Other — no ambiguity, passes through to LLM planner ──
        let _ = token_after; // suppress unused warning
        let _ = rest_after;
        Ok(GuiTaskSpec {
            primary_verb: Verb::Other(
                lower
                    .split_whitespace()
                    .next()
                    .unwrap_or("unknown")
                    .to_string(),
            ),
            targets: vec![],
            content: None,
            declared_preconditions: vec![],
            declared_success_criteria: vec![],
            ambiguities: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::turn_gate::{
        ComputeClass, HazardHint, IntentEnvelope, IntentSource, Modality, Operation,
    };

    fn make_envelope() -> IntentEnvelope {
        IntentEnvelope::new(
            Modality::Text,
            Operation::Automate,
            HazardHint::Green,
            ComputeClass::ToolOnly,
            0.9,
            IntentSource::FastEmbedSemanticRouter,
        )
    }

    #[tokio::test]
    async fn open_app_produces_app_target() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("open firefox", &env)
            .await
            .unwrap();
        assert_eq!(spec.primary_verb, Verb::Open);
        assert_eq!(spec.targets, vec![TargetRef::App("firefox".to_string())]);
        assert!(spec.ambiguities.is_empty());
    }

    #[tokio::test]
    async fn open_url_produces_url_target() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("open https://github.com", &env)
            .await
            .unwrap();
        assert_eq!(spec.primary_verb, Verb::Open);
        assert!(matches!(&spec.targets[0], TargetRef::Url(u) if u.contains("github.com")));
    }

    #[tokio::test]
    async fn navigate_to_produces_url() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("navigate to docs.rs", &env)
            .await
            .unwrap();
        assert_eq!(spec.primary_verb, Verb::Open);
        assert!(matches!(&spec.targets[0], TargetRef::Url(u) if u.contains("docs.rs")));
    }

    #[tokio::test]
    async fn type_quoted_produces_literal() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("type \"hello world\"", &env)
            .await
            .unwrap();
        assert_eq!(spec.primary_verb, Verb::Type);
        assert!(matches!(&spec.content, Some(ContentClass::Literal(s)) if s == "hello world"));
    }

    #[tokio::test]
    async fn click_element_produces_click() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("click Submit button", &env)
            .await
            .unwrap();
        assert_eq!(spec.primary_verb, Verb::Click);
        assert_eq!(
            spec.targets,
            vec![TargetRef::Element("Submit button".to_string())]
        );
    }

    #[tokio::test]
    async fn close_app_produces_close() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("close terminal", &env)
            .await
            .unwrap();
        assert_eq!(spec.primary_verb, Verb::Close);
        assert_eq!(spec.targets, vec![TargetRef::App("terminal".to_string())]);
    }

    #[tokio::test]
    async fn unrecognised_falls_back_to_other_no_error() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("do something complex with the database", &env)
            .await
            .unwrap();
        assert!(matches!(spec.primary_verb, Verb::Other(_)));
        assert!(spec.ambiguities.is_empty());
    }

    #[tokio::test]
    async fn open_with_no_target_requests_clarify() {
        let env = make_envelope();
        let result = RuleIntentCompiler.compile("open", &env).await;
        assert!(result.is_err(), "bare 'open' should request clarification");
    }

    #[tokio::test]
    async fn switch_to_produces_switch() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("switch to code", &env)
            .await
            .unwrap();
        assert_eq!(spec.primary_verb, Verb::Switch);
        assert_eq!(spec.targets, vec![TargetRef::App("code".to_string())]);
    }

    // ── URL detection: false positive prevention ──────────────────────────────

    #[test]
    fn url_detection_rejects_sentence_ending_words() {
        // These caused catastrophic false positives: "output." → "https://output./"
        assert!(!looks_like_url("output."));
        assert!(!looks_like_url("results."));
        assert!(!looks_like_url("loaded."));
        assert!(!looks_like_url("running."));
        assert!(!looks_like_url("available."));
        assert!(!looks_like_url("installed."));
        assert!(!looks_like_url("complete."));
        assert!(!looks_like_url("finished."));
    }

    #[test]
    fn url_detection_rejects_common_abbreviations() {
        assert!(!looks_like_url("e.g."));
        assert!(!looks_like_url("i.e."));
        assert!(!looks_like_url("etc."));
        assert!(!looks_like_url("Mr."));
        assert!(!looks_like_url("Dr."));
    }

    #[test]
    fn url_detection_accepts_real_urls() {
        assert!(looks_like_url("https://example.com"));
        assert!(looks_like_url("http://localhost:3000"));
        assert!(looks_like_url("https://outbro.net"));
        assert!(looks_like_url("www.google.com"));
        assert!(looks_like_url("github.com"));
        assert!(looks_like_url("docs.rs"));
        assert!(looks_like_url("example.org"));
        assert!(looks_like_url("httpbin.org/get"));
    }

    #[test]
    fn url_detection_rejects_short_tokens() {
        assert!(!looks_like_url("a.b"));
        assert!(!looks_like_url("x.y"));
        assert!(!looks_like_url("ok"));
    }

    #[tokio::test]
    async fn sentence_ending_period_not_url() {
        // "show me the output." must NOT produce a URL target
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("show me the output.", &env)
            .await
            .unwrap();
        // Should be Verb::Other, NOT Verb::Open with Url target
        assert!(!spec.targets.iter().any(|t| matches!(t, TargetRef::Url(_))),
            "Sentence-ending 'output.' must not be classified as URL, got: {:?}", spec.targets);
    }

    #[tokio::test]
    async fn real_url_in_sentence_is_detected() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("check out github.com for the code", &env)
            .await
            .unwrap();
        assert!(spec.targets.iter().any(|t| matches!(t, TargetRef::Url(u) if u.contains("github.com"))),
            "Real URL 'github.com' should be detected, got: {:?}", spec.targets);
    }

    // ── Script-write deterministic patterns (no LLM needed) ──────────────────

    #[tokio::test]
    async fn write_python_script_at_path_recognized() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("Write a Python script at /tmp/foo.py that prints hello", &env)
            .await
            .unwrap();
        // Should produce a Verb::Open (file-write workflow)
        assert_eq!(spec.primary_verb, Verb::Open);
        // Should have the file path target
        assert!(spec.targets.iter().any(|t| matches!(t, TargetRef::File(p) if p.to_str().unwrap().contains("foo.py"))),
            "File path target missing: {:?}", spec.targets);
        // Should have generated content with python language
        assert!(matches!(&spec.content, Some(ContentClass::Generated { language: Some(l), .. }) if l == "python"));
    }

    #[tokio::test]
    async fn create_rust_program_recognized() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("Create a Rust program at /tmp/main.rs that calculates fibonacci", &env)
            .await
            .unwrap();
        assert_eq!(spec.primary_verb, Verb::Open);
        assert!(matches!(&spec.content, Some(ContentClass::Generated { language: Some(l), .. }) if l == "rust"));
    }

    #[tokio::test]
    async fn generate_bash_script_recognized() {
        let env = make_envelope();
        let spec = RuleIntentCompiler
            .compile("Generate a bash script at /tmp/x.sh that lists files", &env)
            .await
            .unwrap();
        assert_eq!(spec.primary_verb, Verb::Open);
        assert!(matches!(&spec.content, Some(ContentClass::Generated { language: Some(l), .. }) if l == "bash"));
    }
}
