//! Neutral, schema-driven argument generation (Wave 7.1 / spec R3.4, R9.4).
//!
//! **Relocated from `openclaw::arg_gen`** so that argument-generation cognition
//! lives in the Brain (the neutral capability-intelligence layer), never inside a
//! provider. Providers are pure Hands: they execute a validated request and never
//! decide arguments. This module turns a natural-language request + a
//! capability's JSON `input_schema` into typed, schema-valid arguments GENERALLY
//! — no per-capability logic, no keyword matching, no provider-specific field
//! names — so it works for any current or future capability from ANY provider.
//!
//! Pipeline: read schema → (deterministic fast path handled by the caller) → LLM
//! structured generation → validate against schema → repair/retry → typed args.
//! Never sends invalid arguments to a capability.

use crate::llm::{ChatMessage, LlmBackend};
use serde_json::Value;

use super::super::descriptor::CapabilityDescriptor;
use super::super::error::CapError;

/// Names of the schema's required properties (empty if none / not an object schema).
pub fn required_fields(schema: &Value) -> Vec<String> {
    schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// True when the schema declares an object with at least one property — i.e.
/// there is something to fill. An empty/absent schema means "no args needed".
pub fn schema_expects_arguments(schema: &Value) -> bool {
    schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|o| !o.is_empty())
        .unwrap_or(false)
}

/// Lightweight, dependency-free structural validation against a JSON Schema
/// subset: every required property must be present, non-null, and (when the
/// schema declares a primitive `type`) roughly match that type. Returns the
/// list of offending field names on failure. Intentionally permissive about
/// extra fields ("never send invalid arguments" guard).
pub fn validate_against_schema(args: &Value, schema: &Value) -> Result<(), Vec<String>> {
    let Some(obj) = args.as_object() else {
        return Err(vec!["<root: not a JSON object>".to_string()]);
    };
    let props = schema.get("properties").and_then(|p| p.as_object());
    let mut bad = Vec::new();
    for req in required_fields(schema) {
        match obj.get(&req) {
            None | Some(Value::Null) => bad.push(req),
            Some(v) => {
                if let Some(expected) = props
                    .and_then(|p| p.get(&req))
                    .and_then(|s| s.get("type"))
                    .and_then(|t| t.as_str())
                {
                    if !json_matches_type(v, expected) {
                        bad.push(req);
                    }
                }
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad)
    }
}

fn json_matches_type(v: &Value, expected: &str) -> bool {
    match expected {
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        _ => true, // unknown/union types: don't reject
    }
}

/// Extract the first JSON object from an LLM response, tolerating markdown code
/// fences and surrounding prose.
pub fn extract_json_object(content: &str) -> Option<Value> {
    let trimmed = content.trim();
    if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(trimmed) {
        return Some(v);
    }
    if let Some(stripped) = strip_code_fence(trimmed) {
        if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(stripped.trim()) {
            return Some(v);
        }
    }
    let start = trimmed.find('{')?;
    let mut depth = 0i32;
    let bytes = trimmed.as_bytes();
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let span = &trimmed[start..=i];
                    if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(span) {
                        return Some(v);
                    }
                    break;
                }
            }
            _ => {}
        }
    }
    None
}

fn strip_code_fence(s: &str) -> Option<&str> {
    let s = s.strip_prefix("```")?;
    let s = s.strip_prefix("json").unwrap_or(s);
    let s = s.trim_start_matches(['\n', '\r', ' ']);
    s.strip_suffix("```").or(Some(s))
}

/// Generate typed, schema-valid arguments for a capability from `request`, using
/// `backend`'s strongest structured-output method. Validates against the schema
/// and repairs up to `max_attempts` times. General for any schema / any provider.
pub async fn generate_arguments(
    backend: &dyn LlmBackend,
    capability_id: &str,
    capability_description: &str,
    input_schema: &Value,
    request: &str,
    max_attempts: u32,
) -> Result<Value, String> {
    let schema_pretty = serde_json::to_string_pretty(input_schema).unwrap_or_else(|_| "{}".into());
    let mut feedback = String::new();
    let mut last_err = String::from("no attempt made");

    for _ in 0..max_attempts.max(1) {
        let system = format!(
            "You translate a user request into JSON arguments for a tool call. \
             Respond with ONLY a single JSON object that conforms to the provided \
             JSON Schema — no prose, no markdown, no code fences. Extract values \
             literally from the request; do not invent unrelated data. \
             Tool: {capability_id} — {capability_description}"
        );
        let user = format!(
            "JSON Schema for the arguments:\n{schema_pretty}\n\nUser request:\n{request}{feedback}"
        );
        let messages = [
            ChatMessage {
                role: "system".into(),
                content: system,
                name: None,
                images: None,
            },
            ChatMessage {
                role: "user".into(),
                content: user,
                name: None,
                images: None,
            },
        ];

        let resp = backend
            .chat_structured(&messages, input_schema.clone(), capability_id, 0.0, 512)
            .await
            .map_err(|e| format!("LLM argument generation failed: {e}"))?;

        match extract_json_object(&resp.content) {
            Some(obj) => match validate_against_schema(&obj, input_schema) {
                Ok(()) => return Ok(obj),
                Err(bad) => {
                    last_err = format!("invalid/missing fields: {}", bad.join(", "));
                    feedback = format!(
                        "\n\nYour previous answer was rejected ({last_err}). \
                         Return a corrected JSON object that satisfies the schema."
                    );
                }
            },
            None => {
                last_err = "response was not a JSON object".to_string();
                feedback =
                    "\n\nYour previous answer was not valid JSON. Return ONLY a JSON object."
                        .to_string();
            }
        }
    }

    Err(format!(
        "could not generate schema-valid arguments for '{capability_id}': {last_err}"
    ))
}

/// Default neutral [`ArgumentGenerator`](super::ArgumentGenerator): schema-driven,
/// constrained, provider-agnostic. Wraps a [`LlmBackend`] and produces args from
/// a capability descriptor + goal. Honest: declines (error) rather than
/// fabricating args when the model can't satisfy the schema.
pub struct DefaultArgumentGenerator<B: LlmBackend> {
    backend: B,
    max_attempts: u32,
}

impl<B: LlmBackend> DefaultArgumentGenerator<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            max_attempts: 3,
        }
    }
}

#[async_trait::async_trait]
impl<B: LlmBackend + Send + Sync> super::ArgumentGenerator for DefaultArgumentGenerator<B> {
    async fn generate(
        &self,
        descriptor: &CapabilityDescriptor,
        goal: &str,
    ) -> Result<Value, CapError> {
        if !schema_expects_arguments(&descriptor.input_schema) {
            return Ok(serde_json::json!({}));
        }
        generate_arguments(
            &self.backend,
            &descriptor.capability_id,
            &descriptor.description,
            &descriptor.input_schema,
            goal,
            self.max_attempts,
        )
        .await
        .map_err(CapError::Execute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn calc_schema() -> Value {
        json!({
            "type": "object",
            "properties": { "expression": { "type": "string" } },
            "required": ["expression"]
        })
    }

    #[test]
    fn required_fields_extracted() {
        assert_eq!(
            required_fields(&calc_schema()),
            vec!["expression".to_string()]
        );
    }

    #[test]
    fn schema_expects_arguments_detects_props() {
        assert!(schema_expects_arguments(&calc_schema()));
        assert!(!schema_expects_arguments(
            &json!({"type":"object","properties":{}})
        ));
        assert!(!schema_expects_arguments(&json!({})));
    }

    #[test]
    fn validate_accepts_valid_and_rejects_missing() {
        assert!(validate_against_schema(&json!({"expression":"3+3"}), &calc_schema()).is_ok());
        assert_eq!(
            validate_against_schema(&json!({"query":"3+3"}), &calc_schema()).unwrap_err(),
            vec!["expression".to_string()]
        );
    }

    #[test]
    fn validate_rejects_wrong_type() {
        assert_eq!(
            validate_against_schema(&json!({"expression": 5}), &calc_schema()).unwrap_err(),
            vec!["expression".to_string()]
        );
    }

    #[test]
    fn extract_handles_fences_and_prose() {
        assert_eq!(
            extract_json_object("```json\n{\"expression\":\"3+3\"}\n```").unwrap(),
            json!({"expression":"3+3"})
        );
        assert_eq!(
            extract_json_object("Sure! {\"expression\":\"2*2\"} done").unwrap(),
            json!({"expression":"2*2"})
        );
        assert!(extract_json_object("no json here").is_none());
    }
}
