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

fn looks_like_url(s: &str) -> bool {
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("www.")
        || (s.contains('.') && !s.contains(' ') && s.len() > 4)
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
        for token in text.split_whitespace() {
            if looks_like_url(token) {
                return Ok(GuiTaskSpec {
                    primary_verb: Verb::Open,
                    targets: vec![TargetRef::Url(token.to_string())],
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
}
