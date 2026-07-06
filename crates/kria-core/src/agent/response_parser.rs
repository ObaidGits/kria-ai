use once_cell::sync::Lazy;
use regex::Regex;

/// Parsed tool call from LLM output.
#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

// ─── Tool call regexes ───

/// Pattern 1: XML-style <tool_call>{"name": ..., "arguments": ...}</tool_call>
static TOOL_CALL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)<tool_call>\s*(\{.*?\})\s*</tool_call>").unwrap());

/// Pattern 2: Bracket style [[tool_name(arg1=val1, arg2=val2)]]
static BRACKET_CALL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[\[([A-Za-z0-9_-]+)\(([^)]*)\)\]\]").unwrap());

/// Pattern 3: Raw JSON {"name": "tool_name", "arguments": {...}}
static RAW_JSON_TOOL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"\{"name"\s*:\s*"([A-Za-z0-9_-]+)"\s*,\s*"arguments"\s*:\s*(\{[^}]*\})\}"#)
        .unwrap()
});

/// Pattern 7: Python-style positional call  tool_name("value")  or  tool_name(param="value")
/// Last-resort fallback — only matched when all other patterns fail.
/// The caller must supply a set of known tool names to prevent false positives.
static PYTHON_CALL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?m)^[ \t]*([A-Za-z0-9_-]+)\(([^)]*)\)[ \t]*$"#).unwrap());

/// BUG #4 FIX (category K: Prompt Engineering issue, surfaced through a
/// category D: Dispatcher parsing gap). Root cause: the system prompt's tool-
/// call format instruction shows a literal SCHEMA EXAMPLE —
/// `{"name": "tool_name", "arguments": {"param": "value"}}` — as the format
/// to fill in. When the model has no good real tool for a request (e.g. "CSV
/// to HTML conversion", for which no tool exists), it can echo that template
/// verbatim instead of either declining or naming a real tool. The parser
/// previously accepted ANY string in the `"name"` field with zero validation,
/// so the literal placeholder string reached the dispatcher and produced the
/// confusing "tool 'tool_name' is not available..." error instead of a clear
/// "no matching tool" message. This constant is the literal placeholder from
/// the schema example in `agent/prompts.rs` — never a real, dispatchable tool.
pub const TOOL_CALL_PLACEHOLDER_NAME: &str = "tool_name";

/// True if `name` is the literal schema-template placeholder rather than a
/// real tool name the model intended to call.
fn is_placeholder_tool_name(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case(TOOL_CALL_PLACEHOLDER_NAME)
}

/// Parse all tool calls from LLM output text.
pub fn parse_tool_calls(text: &str) -> Vec<ParsedToolCall> {
    let calls = parse_tool_calls_inner(text);
    // BUG #4 FIX: drop any call whose name is the literal schema placeholder —
    // it is never a real, dispatchable tool and its presence means the model
    // echoed the format example rather than making an intentional tool call.
    calls
        .into_iter()
        .filter(|c| !is_placeholder_tool_name(&c.name))
        .collect()
}

fn parse_tool_calls_inner(text: &str) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();

    // Try Pattern 1: <tool_call>JSON</tool_call>
    for cap in TOOL_CALL_RE.captures_iter(text) {
        if let Some(json_str) = cap.get(1) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str.as_str()) {
                if let (Some(name), Some(args)) = (
                    val.get("name").and_then(|n| n.as_str()),
                    val.get("arguments"),
                ) {
                    calls.push(ParsedToolCall {
                        name: name.to_string(),
                        arguments: args.clone(),
                    });
                }
            }
        }
    }

    if !calls.is_empty() {
        return calls;
    }

    // Try Pattern 2: [[tool_name(args)]]
    for cap in BRACKET_CALL_RE.captures_iter(text) {
        let name = cap
            .get(1)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        let args_str = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        let arguments = parse_kv_args(args_str);
        calls.push(ParsedToolCall { name, arguments });
    }

    if !calls.is_empty() {
        return calls;
    }

    // Try Pattern 3: Raw JSON
    for cap in RAW_JSON_TOOL_RE.captures_iter(text) {
        let name = cap
            .get(1)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        let args_str = cap.get(2).map(|m| m.as_str()).unwrap_or("{}");
        let arguments = serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
        calls.push(ParsedToolCall { name, arguments });
    }

    if !calls.is_empty() {
        return calls;
    }

    calls
}

/// Parse all tool calls from LLM output, using a set of known tool names to enable
/// the Pattern 7 Python-style fallback for single-required-param tools.
/// Pass an empty slice if you don't have registry access (Pattern 7 won't fire).
pub fn parse_tool_calls_with_known(
    text: &str,
    known_tools: &[(&str, &str)], // (tool_name, required_param_name) — single-param tools only
) -> Vec<ParsedToolCall> {
    // Try canonical patterns first
    let calls = parse_tool_calls(text);
    if !calls.is_empty() {
        return calls;
    }

    // Pattern 7: Python-style fallback
    let mut py_calls = Vec::new();
    for cap in PYTHON_CALL_RE.captures_iter(text) {
        let name = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let args_raw = cap.get(2).map(|m| m.as_str().trim()).unwrap_or("");

        // BUG #4 FIX: the Python-style fallback pattern is a last resort and
        // is gated by `known_tools` already, but guard explicitly here too so
        // the placeholder can never slip through even if a caller mistakenly
        // includes it in `known_tools`.
        if is_placeholder_tool_name(name) {
            continue;
        }

        // Only fire if name is a registered single-required-param tool
        let Some((_tool_name, param_name)) = known_tools.iter().find(|(n, _)| *n == name) else {
            continue;
        };

        // Parse args_raw as either positional string literal or key=value
        let arguments = if let Some((key, val)) = args_raw.split_once('=') {
            // key="value" or key='value' style
            let k = key.trim().to_string();
            let v = val.trim().trim_matches('"').trim_matches('\'').to_string();
            serde_json::json!({ k: v })
        } else {
            // positional: just a quoted string
            let v = args_raw.trim_matches('"').trim_matches('\'').to_string();
            if v.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::json!({ *param_name: v })
            }
        };

        py_calls.push(ParsedToolCall {
            name: name.to_string(),
            arguments,
        });
    }

    py_calls
}

/// Parse key=value comma-separated arguments into JSON.
fn parse_kv_args(s: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for part in s.split(',') {
        let trimmed = part.trim();
        if let Some((key, val)) = trimmed.split_once('=') {
            let key = key.trim().to_string();
            let val = val.trim();
            // Try to parse as JSON value, fallback to string
            let json_val = serde_json::from_str(val).unwrap_or_else(|_| {
                let unquoted = val.trim_matches('"').trim_matches('\'');
                serde_json::Value::String(unquoted.to_string())
            });
            map.insert(key, json_val);
        }
    }
    serde_json::Value::Object(map)
}

/// Extract just the text response (non-tool-call parts) from LLM output.
pub fn extract_text_response(text: &str) -> String {
    let mut result = text.to_string();
    // Remove all tool call blocks
    result = TOOL_CALL_RE.replace_all(&result, "").to_string();
    result = BRACKET_CALL_RE.replace_all(&result, "").to_string();
    result = RAW_JSON_TOOL_RE.replace_all(&result, "").to_string();
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_xml_style() {
        let text = r#"Let me check that.
<tool_call>
{"name": "get_cpu_usage", "arguments": {}}
</tool_call>"#;
        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_cpu_usage");
    }

    #[test]
    fn parse_bracket_style() {
        let text = "I'll search for that: [[search_files(directory=/home, pattern=*.txt)]]";
        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search_files");
    }

    #[test]
    fn parse_raw_json_style() {
        let text = r#"{"name": "read_file", "arguments": {"path": "/tmp/test.txt"}}"#;
        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
    }

    #[test]
    fn parse_hyphenated_mcp_tool_name() {
        let text = r#"{"name": "mcp_colab-mcp_execute_cell", "arguments": {"code": "print(1)"}}"#;
        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "mcp_colab-mcp_execute_cell");
    }

    /// BUG #4 regression (category K: Prompt Engineering issue). Reproduces
    /// the exact real production failure: the LLM echoed the system prompt's
    /// literal schema example verbatim (`"name": "tool_name"`) instead of
    /// naming a real tool, e.g. for "Convert this JSON array to CSV" (no
    /// CSV-conversion tool exists). The parser must drop this call entirely
    /// rather than surfacing it to the dispatcher, where it previously
    /// produced the confusing "tool 'tool_name' is not available..." error.
    #[test]
    fn regr_bug4_placeholder_tool_name_is_never_returned_xml_style() {
        let text = r#"<tool_call>
{"name": "tool_name", "arguments": {"param": "value"}}
</tool_call>"#;
        let calls = parse_tool_calls(text);
        assert!(
            calls.is_empty(),
            "literal schema placeholder must be filtered out, got: {calls:?}"
        );
    }

    #[test]
    fn regr_bug4_placeholder_tool_name_is_never_returned_raw_json_style() {
        let text = r#"{"name": "tool_name", "arguments": {"param": "value"}}"#;
        let calls = parse_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn regr_bug4_placeholder_tool_name_is_never_returned_python_style_fallback() {
        let text = "tool_name(\"value\")";
        let known = [("tool_name", "value"), ("real_tool", "value")];
        let calls = parse_tool_calls_with_known(text, &known);
        assert!(
            calls.is_empty(),
            "placeholder must be rejected even if present in known_tools, got: {calls:?}"
        );
    }

    #[test]
    fn regr_bug4_placeholder_check_is_case_insensitive() {
        assert!(is_placeholder_tool_name("Tool_Name"));
        assert!(is_placeholder_tool_name("TOOL_NAME"));
        assert!(is_placeholder_tool_name("  tool_name  "));
        assert!(!is_placeholder_tool_name("real_tool_name"));
        assert!(!is_placeholder_tool_name("hash_text"));
    }

    #[test]
    fn regr_bug4_real_tool_calls_still_parse_normally_alongside_fix() {
        // Non-regression: a genuine call to a real tool must still work.
        let text = r#"{"name": "hash_text", "arguments": {"text": "hi"}}"#;
        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "hash_text");
    }
}
