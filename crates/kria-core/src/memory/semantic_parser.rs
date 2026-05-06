use std::collections::HashMap;
use std::sync::Arc;

use crate::llm::{ChatMessage, LlmBackend};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct MemoryExtraction {
    #[serde(default)]
    pub extracted_facts: Vec<String>,
    #[serde(default)]
    pub user_preferences: HashMap<String, String>,
    #[serde(default)]
    pub inferred_context: String,
}

impl MemoryExtraction {
    pub fn is_empty(&self) -> bool {
        self.extracted_facts.is_empty()
            && self.user_preferences.is_empty()
            && self.inferred_context.trim().is_empty()
    }

    pub fn normalized(mut self) -> Self {
        self.extracted_facts = self
            .extracted_facts
            .into_iter()
            .map(|fact| fact.trim().to_string())
            .filter(|fact| !fact.is_empty())
            .collect();

        self.user_preferences = self
            .user_preferences
            .into_iter()
            .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
            .filter(|(key, value)| !key.is_empty() && !value.is_empty())
            .collect();

        self.inferred_context = self.inferred_context.trim().to_string();
        self
    }
}

/// Semantic memory parser that delegates extraction intelligence to an L1
/// parsing client and returns strictly typed extraction output.
pub struct SemanticMemoryParser {
    parser_client: Arc<dyn LlmBackend>,
    max_tokens: u32,
}

impl SemanticMemoryParser {
    pub fn new(parser_client: Arc<dyn LlmBackend>) -> Self {
        Self {
            parser_client,
            max_tokens: 512,
        }
    }

    pub async fn parse_turn(
        &self,
        user_prompt: &str,
        assistant_response: &str,
    ) -> Option<MemoryExtraction> {
        if user_prompt.trim().is_empty() && assistant_response.trim().is_empty() {
            return None;
        }

        let system_prompt = "You extract durable user memory as strict JSON only. Return exactly one JSON object with keys: extracted_facts (string[]), user_preferences (object string->string), inferred_context (string). Do not include markdown or explanations.";
        let extraction_prompt = format!(
            "User Prompt:\n{user_prompt}\n\nAssistant Response:\n{assistant_response}\n\nReturn JSON now."
        );

        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
                name: None,
                images: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: extraction_prompt,
                name: None,
                images: None,
            },
        ];

        let response = self
            .parser_client
            .chat(&messages, None, 0.0, self.max_tokens)
            .await
            .ok()?;

        let parsed = parse_memory_extraction_payload(&response.content)?;
        let normalized = parsed.normalized();

        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    }
}

fn parse_memory_extraction_payload(raw: &str) -> Option<MemoryExtraction> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(extraction) = serde_json::from_str::<MemoryExtraction>(trimmed) {
        return Some(extraction);
    }

    extract_json_object(trimmed)
        .and_then(|json| serde_json::from_str::<MemoryExtraction>(&json).ok())
}

fn extract_json_object(raw: &str) -> Option<String> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(raw[start..=end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_payload_with_markdown_wrapper() {
        let raw = "```json\n{\"extracted_facts\":[\"user likes tea\"],\"user_preferences\":{\"language\":\"hinglish\"},\"inferred_context\":\"daily routine\"}\n```";
        let parsed = parse_memory_extraction_payload(raw).expect("payload should parse");

        assert_eq!(parsed.extracted_facts.len(), 1);
        assert_eq!(
            parsed
                .user_preferences
                .get("language")
                .expect("language preference must exist"),
            "hinglish"
        );
        assert_eq!(parsed.inferred_context, "daily routine");
    }
}
