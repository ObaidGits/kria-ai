//! LLM-powered semantic intent compiler.
//!
//! Transforms natural-language input into typed [`GuiTaskSpec`] using an LLM.
//! Falls back to `RuleIntentCompiler` for trivially-parseable inputs (<5ms, no LLM call).

use crate::agent::intent_compiler::{
    Ambiguity, ClarifyRequest, ContentClass, GuiTaskSpec, PrereqHint, SuccessHint, TargetRef, Verb,
};
use crate::agent::turn_gate::IntentEnvelope;
use crate::llm::{ChatMessage, LlmBackend};
use std::sync::Arc;

const INTENT_COMPILER_SYSTEM_PROMPT: &str = r#"You are a semantic intent normalizer for a desktop AI assistant called KRIA.

Your job is to extract a structured task specification from natural-language user input.

## Output Format

Output ONLY valid JSON matching this schema. No prose, no markdown.

```json
{
  "primary_verb": "Open|Type|Click|Run|Save|Close|Switch|Other",
  "targets": [{"type": "App|File|Url|Element", "value": "..."}],
  "content": {"type": "Literal|Generated", "text": "...", "language": "python|javascript|..."},
  "declared_preconditions": [{"type": "AppOpen|FileExists|Focused", "value": "..."}],
  "declared_success_criteria": [{"type": "TextInFile|ProcessExited|WindowVisible|UserConfirmed", "path": "...", "substring": "...", "exit_code": 0}],
  "ambiguities": ["AppNotSpecified|FileNotSpecified|MultipleTargetsPossible|ContentScopeUnclear"]
}
```

## Verb Classification

- **Open**: launching apps, opening files/URLs, creating new documents
- **Type**: typing text into fields, filling forms
- **Click**: pressing buttons, selecting menu items, interacting with UI elements
- **Run**: executing code, running scripts, launching processes
- **Save**: saving files, persisting data
- **Close**: closing windows, terminating processes
- **Switch**: switching between apps, tabs, contexts
- **Other**: compound/complex operations (develop, configure, etc.)

## Content Classification

**Literal**: The user explicitly provides the text to type. Examples:
- "type 'hello world'" → Literal("hello world")
- "fill in my email: test@example.com" → Literal("test@example.com")

**Generated**: The user wants content created/calculated. Examples:
- "write a fibonacci program" → Generated{hint: "fibonacci program", language: "python"}
- "solve the problem" → Generated{hint: "problem solution", language: null}
- "create a README" → Generated{hint: "README", language: null}

## Ambiguity Detection

Surface ambiguity when:
- **AppNotSpecified**: "open the editor" (which one?)
- **FileNotSpecified**: "write to the file" (which file?)
- **MultipleTargetsPossible**: "open both VS Code and Firefox" is clear; "open the browser" is ambiguous
- **ContentScopeUnclear**: "run it" after multi-step task (run what how?)

When ambiguity is detected, include it in the `ambiguities` array. The system will ask the user for clarification.

## Examples

Input: "open gedit and type hello world"
```json
{
  "primary_verb": "Open",
  "targets": [{"type": "App", "value": "gedit"}],
  "content": {"type": "Literal", "text": "hello world", "language": null},
  "declared_preconditions": [],
  "declared_success_criteria": [{"type": "WindowVisible", "path": null, "substring": "gedit", "exit_code": null}],
  "ambiguities": []
}
```

Input: "open VS Code and create a fibonacci program and run it"
```json
{
  "primary_verb": "Other",
  "targets": [{"type": "App", "value": "VS Code"}, {"type": "App", "value": "Terminal"}],
  "content": {"type": "Generated", "text": "fibonacci program", "language": "python"},
  "declared_preconditions": [{"type": "AppOpen", "value": "VS Code"}],
  "declared_success_criteria": [{"type": "ProcessExited", "path": null, "substring": null, "exit_code": 0}],
  "ambiguities": ["ContentScopeUnclear"]
}
```

Input: "click the save button"
```json
{
  "primary_verb": "Click",
  "targets": [{"type": "Element", "value": "save button"}],
  "content": null,
  "declared_preconditions": [],
  "declared_success_criteria": [{"type": "UserConfirmed", "path": null, "substring": null, "exit_code": null}],
  "ambiguities": []
}
```

Now process this input:
"#;

/// Get the JSON schema for LLM-constrained output.
/// Uses OnceLock for lazy initialization to avoid const evaluation issues.
fn get_intent_schema() -> &'static serde_json::Value {
    use std::sync::OnceLock;
    static SCHEMA: OnceLock<serde_json::Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::json!({
            "type": "object",
            "properties": {
                "primary_verb": {
                    "type": "string",
                    "enum": ["Open", "Type", "Click", "Run", "Save", "Close", "Switch", "Other"]
                },
                "targets": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "enum": ["App", "File", "Url", "Element"] },
                            "value": { "type": "string" }
                        },
                        "required": ["type", "value"]
                    }
                },
                "content": {
                    "oneOf": [
                        { "type": "null" },
                        {
                            "type": "object",
                            "properties": {
                                "type": { "type": "string", "enum": ["Literal", "Generated"] },
                                "text": { "type": "string" },
                                "language": { "type": ["string", "null"] }
                            },
                            "required": ["type", "text"]
                        }
                    ]
                },
                "declared_preconditions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "enum": ["AppOpen", "FileExists", "Focused"] },
                            "value": { "type": "string" }
                        },
                        "required": ["type", "value"]
                    }
                },
                "declared_success_criteria": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "enum": ["TextInFile", "ProcessExited", "WindowVisible", "UserConfirmed"] },
                            "path": { "type": ["string", "null"] },
                            "substring": { "type": ["string", "null"] },
                            "exit_code": { "type": ["integer", "null"] }
                        },
                        "required": ["type"]
                    }
                },
                "ambiguities": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["AppNotSpecified", "FileNotSpecified", "MultipleTargetsPossible", "ContentScopeUnclear"]
                    }
                }
            },
            "required": ["primary_verb", "targets", "ambiguities"]
        })
    })
}

/// Intermediate JSON structure from LLM (before conversion to GuiTaskSpec).
#[derive(Debug, serde::Deserialize)]
struct LlmIntentSpec {
    primary_verb: String,
    targets: Vec<LlmTarget>,
    #[serde(default)]
    content: Option<LlmContent>,
    #[serde(default)]
    declared_preconditions: Vec<LlmPrereq>,
    #[serde(default)]
    declared_success_criteria: Vec<LlmSuccess>,
    #[serde(default)]
    ambiguities: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct LlmTarget {
    #[serde(rename = "type")]
    target_type: String,
    value: String,
}

#[derive(Debug, serde::Deserialize)]
struct LlmContent {
    #[serde(rename = "type")]
    content_type: String,
    text: String,
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct LlmPrereq {
    #[serde(rename = "type")]
    prereq_type: String,
    value: String,
}

#[derive(Debug, serde::Deserialize)]
struct LlmSuccess {
    #[serde(rename = "type")]
    success_type: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    substring: Option<String>,
    #[serde(default)]
    exit_code: Option<i32>,
}

/// LLM-powered intent compiler.
///
/// Tries rule-based parsing first for trivially-parseable inputs,
/// falls back to LLM for complex inputs.
pub struct LlmIntentCompiler {
    backend: Arc<dyn LlmBackend>,
}

impl LlmIntentCompiler {
    pub fn new(backend: Arc<dyn LlmBackend>) -> Self {
        Self { backend }
    }

    /// Convert LLM JSON output to GuiTaskSpec.
    /// Fails closed on any unknown enum value — never silently normalizes.
    fn convert_llm_output(llm_spec: LlmIntentSpec) -> Result<GuiTaskSpec, ClarifyRequest> {
        let primary_verb = match llm_spec.primary_verb.as_str() {
            "Open" => Verb::Open,
            "Type" => Verb::Type,
            "Click" => Verb::Click,
            "Run" => Verb::Run,
            "Save" => Verb::Save,
            "Close" => Verb::Close,
            "Switch" => Verb::Switch,
            other => {
                return Err(ClarifyRequest {
                    question: format!("Malformed intent classification: unknown verb '{}'", other),
                    options: vec!["Try rephrasing".to_string()],
                });
            }
        };

        let mut targets = Vec::new();
        for t in llm_spec.targets {
            let target = match t.target_type.as_str() {
                "App" => TargetRef::App(t.value),
                "File" => TargetRef::File(std::path::PathBuf::from(t.value)),
                "Url" => TargetRef::Url(t.value),
                "Element" => TargetRef::Element(t.value),
                other => {
                    return Err(ClarifyRequest {
                        question: format!(
                            "Malformed intent classification: unknown target type '{}'",
                            other
                        ),
                        options: vec!["Try rephrasing".to_string()],
                    });
                }
            };
            targets.push(target);
        }

        let content = if let Some(c) = llm_spec.content {
            Some(match c.content_type.as_str() {
                "Literal" => ContentClass::Literal(c.text),
                "Generated" => ContentClass::Generated {
                    hint: c.text,
                    language: c.language,
                },
                other => {
                    return Err(ClarifyRequest {
                        question: format!(
                            "Malformed intent classification: unknown content type '{}'",
                            other
                        ),
                        options: vec!["Try rephrasing".to_string()],
                    });
                }
            })
        } else {
            None
        };

        let mut declared_preconditions = Vec::new();
        for p in llm_spec.declared_preconditions {
            let prereq = match p.prereq_type.as_str() {
                "AppOpen" => PrereqHint::AppOpen(p.value),
                "FileExists" => PrereqHint::FileExists(std::path::PathBuf::from(p.value)),
                "Focused" => {
                    let target = match p.value.split_once(':') {
                        Some(("App", v)) => TargetRef::App(v.to_string()),
                        Some(("File", v)) => TargetRef::File(std::path::PathBuf::from(v)),
                        Some(("Url", v)) => TargetRef::Url(v.to_string()),
                        _ => TargetRef::Element(p.value),
                    };
                    PrereqHint::Focused(target)
                }
                other => {
                    return Err(ClarifyRequest {
                        question: format!(
                            "Malformed intent classification: unknown precondition type '{}'",
                            other
                        ),
                        options: vec!["Try rephrasing".to_string()],
                    });
                }
            };
            declared_preconditions.push(prereq);
        }

        let mut declared_success_criteria = Vec::new();
        for s in llm_spec.declared_success_criteria {
            let criterion = match s.success_type.as_str() {
                "TextInFile" => SuccessHint::TextInFile {
                    path: std::path::PathBuf::from(s.path.unwrap_or_default()),
                    substring: s.substring.unwrap_or_default(),
                },
                "ProcessExited" => SuccessHint::ProcessExited(s.exit_code.unwrap_or(0) as u32),
                "WindowVisible" => SuccessHint::WindowVisible(s.substring.unwrap_or_default()),
                "UserConfirmed" => SuccessHint::UserConfirmed,
                other => {
                    return Err(ClarifyRequest {
                        question: format!(
                            "Malformed intent classification: unknown success criterion '{}'",
                            other
                        ),
                        options: vec!["Try rephrasing".to_string()],
                    });
                }
            };
            declared_success_criteria.push(criterion);
        }

        let mut ambiguities = Vec::new();
        for a in llm_spec.ambiguities {
            let ambiguity = match a.as_str() {
                "AppNotSpecified" => Ambiguity::AppNotSpecified,
                "FileNotSpecified" => Ambiguity::FileNotSpecified,
                "MultipleTargetsPossible" => Ambiguity::MultipleTargetsPossible,
                "ContentScopeUnclear" => Ambiguity::ContentScopeUnclear,
                other => {
                    return Err(ClarifyRequest {
                        question: format!(
                            "Malformed intent classification: unknown ambiguity '{}'",
                            other
                        ),
                        options: vec!["Try rephrasing".to_string()],
                    });
                }
            };
            ambiguities.push(ambiguity);
        }

        Ok(GuiTaskSpec {
            primary_verb,
            targets,
            content,
            declared_preconditions,
            declared_success_criteria,
            ambiguities,
        })
    }

    /// Build clarification request from ambiguities.
    fn build_clarify_request(ambiguities: &[Ambiguity]) -> ClarifyRequest {
        use std::fmt::Write;
        let mut question = String::new();
        let mut options = Vec::new();

        for ambiguity in ambiguities {
            match ambiguity {
                Ambiguity::AppNotSpecified => {
                    let _ = write!(&mut question, "Which application should I use? ");
                    options.push("gedit (text editor)".to_string());
                    options.push("VS Code (code editor)".to_string());
                    options.push("Firefox (browser)".to_string());
                    options.push("Terminal".to_string());
                }
                Ambiguity::FileNotSpecified => {
                    let _ = write!(&mut question, "Which file should I use? ");
                    options.push("Create a new file".to_string());
                    options.push("Use a specific file (provide path)".to_string());
                }
                Ambiguity::MultipleTargetsPossible => {
                    let _ = write!(&mut question, "Which one should I use? ");
                    options.push("First option".to_string());
                    options.push("Second option".to_string());
                    options.push("Both".to_string());
                }
                Ambiguity::ContentScopeUnclear => {
                    let _ = write!(&mut question, "How would you like to run this? ");
                    options.push("Run in terminal".to_string());
                    options.push("Run with F5/Run button".to_string());
                    options.push("Save and let me run it manually".to_string());
                }
            }
        }

        ClarifyRequest { question, options }
    }

    /// Call LLM with grammar-constrained JSON schema and parse response.
    async fn call_llm(&self, user_text: &str) -> Result<GuiTaskSpec, ClarifyRequest> {
        let trimmed = user_text.trim();
        let messages = vec![
            ChatMessage {
                role: "system".into(),
                content: INTENT_COMPILER_SYSTEM_PROMPT.to_string(),
                name: None,
                images: None,
            },
            ChatMessage {
                role: "user".into(),
                content: trimmed.to_string(),
                name: None,
                images: None,
            },
        ];

        // GBNF-constrained call: enforce closed enum output via JSON schema
        let schema = get_intent_schema().clone();
        let response = self
            .backend
            .chat_with_grammar(&messages, schema, 0.1, 512)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "LLM grammar chat failed");
                ClarifyRequest {
                    question: format!("I couldn't understand: {}", trimmed),
                    options: vec![
                        "Try a simpler command".to_string(),
                        "Open an application".to_string(),
                        "Type some text".to_string(),
                    ],
                }
            })?;

        let json_str = response.content.trim();
        let llm_spec: LlmIntentSpec = serde_json::from_str(json_str).map_err(|e| {
            tracing::warn!(error = %e, json = %json_str[..json_str.len().min(200)], "failed to parse LLM intent JSON");
            ClarifyRequest {
                question: "I couldn't parse the intent. Could you rephrase?".to_string(),
                options: vec![
                    "Try a simpler command".to_string(),
                    "Open an application".to_string(),
                    "Type some text".to_string(),
                ],
            }
        })?;

        let spec = Self::convert_llm_output(llm_spec)?;

        if !spec.ambiguities.is_empty() {
            return Err(Self::build_clarify_request(&spec.ambiguities));
        }

        Ok(spec)
    }
}

#[async_trait::async_trait]
impl super::intent_compiler::IntentCompiler for LlmIntentCompiler {
    async fn compile(
        &self,
        user_text: &str,
        _intent: &IntentEnvelope,
    ) -> Result<GuiTaskSpec, ClarifyRequest> {
        let trimmed = user_text.trim();
        if trimmed.is_empty() {
            return Err(ClarifyRequest {
                question: "What would you like me to do?".to_string(),
                options: vec![
                    "Open an application".to_string(),
                    "Type some text".to_string(),
                    "Click a button".to_string(),
                    "Run a command".to_string(),
                ],
            });
        }

        // Fast path: try rule-based parsing first
        let intent = crate::agent::turn_gate::IntentEnvelope::new(
            crate::agent::turn_gate::Modality::Text,
            crate::agent::turn_gate::Operation::Automate,
            crate::agent::turn_gate::HazardHint::Green,
            crate::agent::turn_gate::ComputeClass::L1Text,
            0.9,
            crate::agent::turn_gate::IntentSource::DeterministicGuard,
        );
        if let Ok(spec) = RuleIntentCompiler.compile(trimmed, &intent).await {
            if spec.ambiguities.is_empty() {
                tracing::debug!(verb = ?spec.primary_verb, targets = ?spec.targets, "intent compiled via rule parser");
                return Ok(spec);
            }
        }

        // Slow path: LLM for complex inputs
        tracing::info!(input_len = trimmed.len(), "intent requires LLM parsing");
        self.call_llm(trimmed).await
    }
}

/// Rule-based intent compiler for trivially-parseable inputs.
///
/// Handles common patterns like "open <app>", "type <text>", "click <element>".
/// This is a fast path (<5ms, no LLM) for unambiguous inputs.
pub struct RuleIntentCompiler;

impl RuleIntentCompiler {
    fn parse_verb(text: &str) -> Option<Verb> {
        let lower = text.to_ascii_lowercase();

        if lower.starts_with("open ")
            || lower.starts_with("launch ")
            || lower.starts_with("navigate to ")
            || lower.starts_with("go to ")
        {
            Some(Verb::Open)
        } else if lower.starts_with("type ")
            || lower.starts_with("type '")
            || lower.starts_with("type \"")
        {
            Some(Verb::Type)
        } else if lower.starts_with("click ") || lower.starts_with("press ") {
            Some(Verb::Click)
        } else if lower.starts_with("run ") || lower.starts_with("execute ") {
            Some(Verb::Run)
        } else if lower.starts_with("save ") {
            Some(Verb::Save)
        } else if lower.starts_with("close ") || lower.starts_with("quit ") {
            Some(Verb::Close)
        } else if lower.starts_with("switch ") || lower.starts_with("switch to ") {
            Some(Verb::Switch)
        } else {
            None
        }
    }

    fn extract_app(text: &str) -> Option<String> {
        let lower = text.to_ascii_lowercase();

        // Common editors
        if lower.contains("gedit") || lower.contains("text editor") {
            return Some("gedit".to_string());
        }
        if lower.contains("vscode")
            || lower.contains("vs code")
            || lower.contains("visual studio code")
        {
            return Some("code".to_string());
        }
        // Bare "code" alias — matches whole-word "code" only (avoids "decode",
        // "encode", "barcode", etc.). Matches "open code", "launch code and …".
        if Self::contains_word(&lower, "code") {
            return Some("code".to_string());
        }
        if lower.contains("sublime") {
            return Some("subl".to_string());
        }
        if lower.contains("firefox") {
            return Some("firefox".to_string());
        }
        if lower.contains("chrome") {
            return Some("google-chrome".to_string());
        }
        if lower.contains("brave") {
            return Some("brave-browser".to_string());
        }
        // "browser" without a specific name → use xdg-open default browser
        // rather than hardcoding firefox (user may have a different default)
        if lower.contains("browser") && !lower.contains("firefox") && !lower.contains("chrome") {
            return Some("xdg-open".to_string());
        }
        if lower.contains("terminal")
            || lower.contains("konsole")
            || lower.contains("gnome-terminal")
        {
            return Some("gnome-terminal".to_string());
        }
        if lower.contains("nautilus") || lower.contains("file manager") {
            return Some("nautilus".to_string());
        }

        // Generic "open <X>" extraction. Stops at sentence connectors so
        // "open code and write a program" doesn't yield "code and".
        if let Some(pos) = lower.rfind("open ").or_else(|| lower.rfind("launch ")) {
            // Skip whichever verb matched
            let verb_len = if lower[pos..].starts_with("open ") {
                5
            } else {
                7
            };
            let after = text[pos + verb_len..].trim();
            // Tokenize and collect words until we hit a connector
            const STOP_TOKENS: &[&str] = &["and", "then", "&", ",", ";", "to", "with", "in", "for"];
            let mut tokens: Vec<&str> = Vec::new();
            for tok in after.split_whitespace() {
                let lower_tok = tok.to_ascii_lowercase();
                let stripped = lower_tok.trim_end_matches([',', ';', '.', ':']);
                if STOP_TOKENS.contains(&stripped) {
                    break;
                }
                tokens.push(tok);
                if tokens.len() >= 3 {
                    break;
                }
            }
            // Drop leading articles
            while let Some(first) = tokens.first() {
                let lf = first.to_ascii_lowercase();
                if matches!(lf.as_str(), "the" | "a" | "an" | "my") {
                    tokens.remove(0);
                } else {
                    break;
                }
            }
            let app = tokens.join(" ");
            // Reject generic placeholders that should trigger a clarify path
            // ("the editor", "the browser", "an app"). These are handled by
            // the named-app branches above when the user is specific.
            const AMBIGUOUS_NOUNS: &[&str] =
                &["editor", "app", "application", "thing", "something", "it"];
            if !app.is_empty() && !AMBIGUOUS_NOUNS.contains(&app.to_ascii_lowercase().as_str()) {
                return Some(app);
            }
        }

        None
    }

    /// Whole-word containment check (no substring matching across word
    /// boundaries). Used so "code" doesn't match "decode"/"barcode".
    fn contains_word(haystack: &str, needle: &str) -> bool {
        haystack
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|w| w == needle)
    }

    fn extract_literal_text(text: &str) -> Option<String> {
        let lower = text.to_ascii_lowercase();

        // Pattern: type 'text' or type "text"
        if let Some(start) = lower.find("type '") {
            let after = &text[start + 6..];
            if let Some(end) = after.find('\'') {
                return Some(after[..end].to_string());
            }
        }
        if let Some(start) = lower.find("type \"") {
            let after = &text[start + 6..];
            if let Some(end) = after.find('"') {
                return Some(after[..end].to_string());
            }
        }

        // Pattern: type <text> (without quotes) - take everything after "type "
        if let Some(pos) = lower.find("type ") {
            let after = text[pos + 5..].trim();
            // FIX #16: "hello world" was incorrectly excluded from literal detection.
            // "type hello world" IS a literal typing request — remove it from the exclusion list.
            let is_literal = !["a ", "the ", "some "]
                .iter()
                .any(|kw| after.starts_with(kw));
            let no_generation_hints = ![
                "fibonacci",
                "program",
                "code",
                "script",
                "function",
                "algorithm",
            ]
            .iter()
            .any(|kw| after.contains(kw));

            if is_literal && no_generation_hints && after.len() > 0 && after.len() < 500 {
                return Some(after.to_string());
            }
        }

        None
    }

    /// Strip common "the following [label]: " prefixes so "and type the following poem: Roses..."
    /// yields "Roses..." as the literal content.
    fn strip_following_prefix(text: &str) -> String {
        let t = text.trim();
        let lower = t.to_ascii_lowercase();
        // "the following: content" — colon immediately after "following"
        if lower.starts_with("the following:") {
            return t["the following:".len()..].trim().to_string();
        }
        // "the following [label]: content" — short label then colon
        if lower.starts_with("the following ") {
            let after = &t["the following ".len()..];
            if let Some(colon_pos) = after.find(':') {
                let label = &after[..colon_pos];
                if label.split_whitespace().count() <= 3 {
                    return after[colon_pos + 1..].trim().to_string();
                }
            }
            return after.trim().to_string();
        }
        t.to_string()
    }

    fn is_generated_content(text: &str) -> bool {
        let lower = text.to_ascii_lowercase();
        let generation_keywords = [
            "fibonacci",
            "program",
            "code",
            "script",
            "function",
            "algorithm",
            "solve",
            "implement",
            "create a",
            "write a",
            "generate",
            "build a",
        ];
        generation_keywords.iter().any(|kw| lower.contains(kw))
    }

    /// Extract the stem (name without extension) from explicit filename patterns:
    /// "called notes.txt" → "notes"
    /// "named README.md"  → "readme"
    /// "save as hello.py" → "hello"
    /// "name it config.json" → "config"
    fn extract_explicit_filename_stem(text: &str) -> Option<String> {
        let lower = text.to_ascii_lowercase();
        let markers = [
            "called ",
            "named ",
            "name it ",
            "save it as ",
            "save as ",
            "filename ",
            "file name ",
        ];
        for marker in &markers {
            if let Some(pos) = lower.find(marker) {
                let after = text[pos + marker.len()..].trim();
                let token = after.split_whitespace().next()?;
                let clean: String = token
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
                    .collect();
                if clean.is_empty() {
                    continue;
                }
                let stem = if let Some(dot) = clean.rfind('.') {
                    clean[..dot].to_ascii_lowercase()
                } else {
                    clean.to_ascii_lowercase()
                };
                if stem.len() >= 2 {
                    return Some(stem);
                }
            }
        }
        None
    }

    fn extract_all_urls(text: &str) -> Vec<String> {
        let mut urls = Vec::new();
        let clean_text = text.replace(',', " ").replace(';', " ").replace('!', " ");
        for word in clean_text.split_whitespace() {
            // Skip email addresses — they contain '@' and a domain but are not URLs
            if word.contains('@') {
                continue;
            }
            let cleaned =
                word.trim_matches(|c: char| c == '\'' || c == '"' || c == '.' || c == '/');
            let cleaned_lower = cleaned.to_ascii_lowercase();
            if cleaned_lower.starts_with("http://")
                || cleaned_lower.starts_with("https://")
                || cleaned_lower.contains(".com")
                || cleaned_lower.contains(".org")
                || cleaned_lower.contains(".net")
                || cleaned_lower.contains("localhost")
            {
                urls.push(cleaned.to_string());
            }
        }
        urls
    }
}

#[async_trait::async_trait]
impl super::intent_compiler::IntentCompiler for RuleIntentCompiler {
    async fn compile(
        &self,
        user_text: &str,
        _intent: &IntentEnvelope,
    ) -> Result<GuiTaskSpec, ClarifyRequest> {
        let trimmed = user_text.trim();
        let lower = trimmed.to_ascii_lowercase();

        let primary_verb = Self::parse_verb(trimmed).unwrap_or(Verb::Other(trimmed.to_string()));

        let mut targets = Vec::new();
        let mut content = None;
        let declared_preconditions = Vec::new();
        let mut declared_success_criteria = Vec::new();
        let ambiguities = Vec::new();

        match primary_verb {
            Verb::Open => {
                let urls = Self::extract_all_urls(trimmed);
                if !urls.is_empty() {
                    for url in urls {
                        targets.push(TargetRef::Url(url));
                    }
                } else if let Some(app) = Self::extract_app(trimmed) {
                    targets.push(TargetRef::App(app));
                } else {
                    // Ambiguous - no app specified
                    return Err(ClarifyRequest {
                        question: "Which application should I open?".to_string(),
                        options: vec![
                            "gedit (text editor)".to_string(),
                            "VS Code (code editor)".to_string(),
                            "Firefox (browser)".to_string(),
                            "Terminal".to_string(),
                        ],
                    });
                }
                declared_success_criteria.push(SuccessHint::WindowVisible("Open".to_string()));

                // Compound prompt detection: "open X and type/write/create/generate/implement/build Y".
                // We treat the second clause's content the same way `Verb::Type`
                // does so the substrate planner can pick the file path.
                // Literal-only markers: "and type: X" and ", type X" always produce
                // ContentClass::Literal regardless of is_generated_content.
                let literal_only_markers = [" and type:", ", type ", ", type: "];
                let mut found_literal = false;
                for marker in &literal_only_markers {
                    if let Some(pos) = lower.find(marker) {
                        let after = trimmed[pos + marker.len()..].trim();
                        let literal = Self::strip_following_prefix(after);
                        if !literal.is_empty() {
                            content = Some(ContentClass::Literal(literal));
                            found_literal = true;
                            break;
                        }
                    }
                }

                let compound_markers = [
                    " and type ",
                    " and write ",
                    " and create ",
                    " and generate ",
                    " and implement ",
                    " and build ",
                    " and develop ",
                    " and code ",
                    " and make ",
                ];
                if !found_literal {
                    if let Some(rest) = compound_markers
                        .iter()
                        .find_map(|marker| lower.find(marker).map(|i| (i, marker.len())))
                        .map(|(i, mlen)| trimmed[i + mlen..].trim().to_string())
                    {
                        if !rest.is_empty() {
                            if Self::is_generated_content(&rest)
                                || Self::is_generated_content(trimmed)
                            {
                                // Extract a meaningful hint — take up to 15 words,
                                // skipping leading articles. This ensures topic words
                                // like "fibonacci" or "lines" appear even in long prompts.
                                let content_hint = rest
                                    .split_whitespace()
                                    .skip_while(|w| ["a", "an", "the"].contains(w))
                                    .take(15)
                                    .collect::<Vec<_>>()
                                    .join(" ");

                                // If the user specified an explicit filename ("called notes.txt",
                                // "named README.md", "save as hello.py", etc.), prepend its stem
                                // to the hint so generate_filename uses it rather than the first
                                // content word (e.g. "shopping").
                                let explicit_stem = Self::extract_explicit_filename_stem(trimmed);
                                let hint = match explicit_stem {
                                    Some(stem) => format!("{} {}", stem, content_hint),
                                    None => content_hint,
                                };
                                // If the hint names a non-code file type (.md, README, JSON,
                                // HTML, shell script) treat language as None so the planner
                                // picks the correct extension and content generator.
                                let is_doc_file = lower.contains("readme")
                                    || lower.contains(".md")
                                    || lower.contains("markdown")
                                    || lower.contains("config.json")
                                    || lower.contains(".json")
                                    || lower.contains("html")
                                    || lower.contains(".html")
                                    || lower.contains("shell script")
                                    || lower.contains("bash script")
                                    || lower.contains(".sh");
                                let language = if is_doc_file {
                                    None
                                } else if lower.contains("python") {
                                    Some("python".to_string())
                                } else if lower.contains("javascript")
                                    || lower.split_whitespace().any(|w| w == "js" || w == "js,")
                                    || lower.contains("node.js")
                                    || lower.contains("nodejs")
                                {
                                    Some("javascript".to_string())
                                } else if lower.contains("typescript") {
                                    Some("typescript".to_string())
                                } else if lower.contains("rust") {
                                    Some("rust".to_string())
                                } else if lower.contains("golang")
                                    || lower.contains(" go ")
                                    || lower.ends_with(" go")
                                    || lower.contains(" go\n")
                                    || lower.contains("go language")
                                {
                                    Some("go".to_string())
                                } else if lower.contains("java") && !lower.contains("javascript") {
                                    Some("java".to_string())
                                } else if lower.contains("kotlin") {
                                    Some("kotlin".to_string())
                                } else if lower.contains("ruby") {
                                    Some("ruby".to_string())
                                } else if lower.contains(" php") || lower.ends_with("php") {
                                    Some("php".to_string())
                                } else if lower.contains("c#") || lower.contains("csharp") {
                                    Some("csharp".to_string())
                                } else if lower.contains("c++") || lower.contains("cpp") {
                                    Some("cpp".to_string())
                                } else if lower.contains("swift") {
                                    Some("swift".to_string())
                                } else if lower.contains("bash") || lower.contains("shell script") {
                                    Some("bash".to_string())
                                } else {
                                    None
                                };
                                content = Some(ContentClass::Generated { hint, language });
                            } else if let Some(literal) = Self::extract_literal_text(&rest) {
                                content = Some(ContentClass::Literal(literal));
                            } else {
                                // Fallback: use rest directly as literal after stripping prefix
                                let literal = Self::strip_following_prefix(&rest);
                                if !literal.is_empty() {
                                    content = Some(ContentClass::Literal(literal));
                                }
                            }
                        }
                    }
                } // end !found_literal
            }
            Verb::Type => {
                if let Some(text) = Self::extract_literal_text(trimmed) {
                    content = Some(ContentClass::Literal(text));
                } else if Self::is_generated_content(trimmed) {
                    // Detect generated content
                    let hint = trimmed
                        .split_whitespace()
                        .skip_while(|w| !["a", "an"].contains(w))
                        .skip(1)
                        .take(5)
                        .collect::<Vec<_>>()
                        .join(" ");

                    let language = if lower.contains("python") {
                        Some("python".to_string())
                    } else if lower.contains("javascript")
                        // FIX #28: Use word-boundary check for bare "js" to avoid
                        // false positives: "objects", "adjusts", "projects" all contain "js"
                        || lower.split_whitespace().any(|w| w == "js" || w == "js,")
                    {
                        Some("javascript".to_string())
                    } else if lower.contains("rust") {
                        Some("rust".to_string())
                    } else {
                        None
                    };

                    content = Some(ContentClass::Generated { hint, language });
                } else {
                    return Err(ClarifyRequest {
                        question: "What text should I type?".to_string(),
                        options: vec![
                            "Type 'hello world'".to_string(),
                            "Type specific text (provide it)".to_string(),
                        ],
                    });
                }
            }
            Verb::Click => {
                let element = trimmed
                    .strip_prefix("click ")
                    .or_else(|| trimmed.strip_prefix("press "))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| trimmed.to_string());

                if element.is_empty() {
                    return Err(ClarifyRequest {
                        question: "Which element should I click?".to_string(),
                        options: vec![
                            "save button".to_string(),
                            "submit button".to_string(),
                            "ok button".to_string(),
                            "cancel button".to_string(),
                        ],
                    });
                }

                targets.push(TargetRef::Element(element));
            }
            Verb::Run => {
                if let Some(app) = Self::extract_app(trimmed) {
                    targets.push(TargetRef::App(app));
                }
            }
            _ => {
                // Other verbs - just pass through
            }
        }

        Ok(GuiTaskSpec {
            primary_verb,
            targets,
            content,
            declared_preconditions,
            declared_success_criteria,
            ambiguities,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::intent_compiler::IntentCompiler;
    use crate::agent::turn_gate::{ComputeClass, HazardHint, IntentSource, Modality, Operation};

    fn make_intent() -> IntentEnvelope {
        IntentEnvelope::new(
            Modality::Text,
            Operation::Automate,
            HazardHint::Green,
            ComputeClass::L1Text,
            0.9,
            IntentSource::FastEmbedSemanticRouter,
        )
    }

    #[tokio::test]
    async fn test_rule_compiler_open_gedit() {
        let compiler = RuleIntentCompiler;
        let spec = compiler
            .compile("open gedit", &make_intent())
            .await
            .unwrap();
        assert!(matches!(spec.primary_verb, Verb::Open));
        assert_eq!(spec.targets.len(), 1);
        assert!(matches!(spec.targets[0], TargetRef::App(ref s) if s.contains("gedit")));
    }

    #[tokio::test]
    async fn test_rule_compiler_type_literal() {
        let compiler = RuleIntentCompiler;
        let spec = compiler
            .compile("type 'hello world'", &make_intent())
            .await
            .unwrap();
        assert!(matches!(spec.primary_verb, Verb::Type));
        match &spec.content {
            Some(ContentClass::Literal(text)) => assert_eq!(text, "hello world"),
            other => panic!("Expected Literal, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_rule_compiler_type_generated() {
        let compiler = RuleIntentCompiler;
        let spec = compiler
            .compile("type a fibonacci program", &make_intent())
            .await
            .unwrap();
        assert!(matches!(spec.primary_verb, Verb::Type));
        match &spec.content {
            Some(ContentClass::Generated { hint, language }) => {
                assert!(hint.contains("fibonacci") || hint.contains("program"));
                assert!(language.is_none() || language.as_deref() == Some("python"));
            }
            other => panic!("Expected Generated, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_rule_compiler_click_button() {
        let compiler = RuleIntentCompiler;
        let spec = compiler
            .compile("click the save button", &make_intent())
            .await
            .unwrap();
        assert!(matches!(spec.primary_verb, Verb::Click));
        assert_eq!(spec.targets.len(), 1);
        assert!(matches!(&spec.targets[0], TargetRef::Element(e) if e.contains("save")));
    }

    #[tokio::test]
    async fn test_rule_compiler_ambiguous_app() {
        let compiler = RuleIntentCompiler;
        let result = compiler.compile("open the editor", &make_intent()).await;
        assert!(result.is_err()); // Should raise clarification
        let err = result.unwrap_err();
        assert!(err.question.contains("Which application"));
    }

    #[test]
    fn test_llm_compiler_converts_output() {
        // This is a unit test of the conversion logic without needing an LLM
        let llm_spec = LlmIntentSpec {
            primary_verb: "Open".to_string(),
            targets: vec![LlmTarget {
                target_type: "App".to_string(),
                value: "gedit".to_string(),
            }],
            content: Some(LlmContent {
                content_type: "Literal".to_string(),
                text: "hello".to_string(),
                language: None,
            }),
            declared_preconditions: vec![],
            declared_success_criteria: vec![LlmSuccess {
                success_type: "WindowVisible".to_string(),
                path: None,
                substring: Some("gedit".to_string()),
                exit_code: None,
            }],
            ambiguities: vec![],
        };

        let spec = LlmIntentCompiler::convert_llm_output(llm_spec).unwrap();
        assert!(matches!(spec.primary_verb, Verb::Open));
        assert_eq!(spec.targets.len(), 1);
    }

    #[tokio::test]
    async fn test_rule_compiler_compound_open_and_type_generates_content() {
        let compiler = RuleIntentCompiler;
        let spec = compiler
            .compile(
                "open gedit and type a program to print fibonacci series in python",
                &make_intent(),
            )
            .await
            .unwrap();
        assert!(matches!(spec.primary_verb, Verb::Open));
        assert_eq!(spec.targets.len(), 1);
        assert!(matches!(&spec.targets[0], TargetRef::App(s) if s.contains("gedit")));
        match &spec.content {
            Some(ContentClass::Generated { hint, language }) => {
                assert!(
                    hint.contains("program") || hint.contains("fibonacci"),
                    "hint should reflect content: {}",
                    hint
                );
                assert_eq!(language.as_deref(), Some("python"));
            }
            other => panic!("Expected Generated content, got {:?}", other),
        }
    }

    /// Regression: "Open code and write a program to print pascals triangle in
    /// python3 and run the program" must NOT extract `code and` as the app
    /// name. The conjunction-aware extractor stops at "and".
    #[tokio::test]
    async fn regression_open_code_does_not_eat_conjunction() {
        let compiler = RuleIntentCompiler;
        let spec = compiler
            .compile(
                "Open code and Write a program to print pascals triangle in python3 and run the prgram",
                &make_intent(),
            )
            .await
            .unwrap();
        assert!(matches!(spec.primary_verb, Verb::Open));
        match &spec.targets[0] {
            TargetRef::App(s) => assert_eq!(s, "code", "expected app to be 'code', got '{}'", s),
            other => panic!("Expected App target, got {:?}", other),
        }
        match &spec.content {
            Some(ContentClass::Generated { language, .. }) => {
                assert_eq!(language.as_deref(), Some("python"));
            }
            other => panic!("Expected Generated content, got {:?}", other),
        }
    }

    /// Regression: "Open Code and …" (capitalised) must also resolve to bare
    /// "code" and not "Code and".
    #[tokio::test]
    async fn regression_open_code_capitalised_does_not_eat_conjunction() {
        let compiler = RuleIntentCompiler;
        let spec = compiler
            .compile(
                "Open Code and Write a program to print pascals triangle in python3 and run the prgram",
                &make_intent(),
            )
            .await
            .unwrap();
        match &spec.targets[0] {
            TargetRef::App(s) => assert_eq!(s, "code"),
            other => panic!("Expected App, got {:?}", other),
        }
    }

    /// Regression: "open chrome and search for youtube" must extract just
    /// "chrome" / "google-chrome", not "chrome and".
    #[tokio::test]
    async fn regression_open_chrome_extracts_chrome_only() {
        let compiler = RuleIntentCompiler;
        let spec = compiler
            .compile("open chrome and search for youtube", &make_intent())
            .await
            .unwrap();
        match &spec.targets[0] {
            TargetRef::App(s) => assert!(
                s.contains("chrome") && !s.contains("and"),
                "expected chrome alias, got '{}'",
                s
            ),
            other => panic!("Expected App, got {:?}", other),
        }
    }

    #[test]
    fn contains_word_does_not_match_substrings() {
        // "decode" / "encode" should not yield a "code" hit
        assert!(!RuleIntentCompiler::contains_word(
            "please decode this",
            "code"
        ));
        assert!(!RuleIntentCompiler::contains_word(
            "encode the message",
            "code"
        ));
        assert!(!RuleIntentCompiler::contains_word("barcode reader", "code"));
        // But standalone "code" should match
        assert!(RuleIntentCompiler::contains_word("open code now", "code"));
        assert!(RuleIntentCompiler::contains_word(
            "launch code, please",
            "code"
        ));
    }
}
