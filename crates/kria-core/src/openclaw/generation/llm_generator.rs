//! Production `SkillGenerator` backed by an `LlmBackend`.
//!
//! Uses grammar-constrained JSON where the backend supports it. Parsing is defensive:
//! malformed JSON surfaces as `GeneratorError::Parse` so the pipeline can repair/retry.
//! This is the ONE production generator — no parallel generation path (A9.15).

use super::designer::{classify_risk, infer_capabilities, SkillDesign, SkillExample};
use super::generator::{GeneratedArtifacts, GeneratorError, SkillGenerator};
use super::requirements::SkillRequirement;
use crate::llm::{ChatMessage, LlmBackend};
use async_trait::async_trait;
use std::sync::Arc;

/// LLM-backed skill generator (A9.2/A9.3/A9.5/A9.8).
pub struct LlmSkillGenerator {
    backend: Arc<dyn LlmBackend>,
    temperature: f32,
    max_tokens: u32,
}

impl LlmSkillGenerator {
    pub fn new(backend: Arc<dyn LlmBackend>) -> Self {
        Self {
            backend,
            temperature: 0.2,
            max_tokens: 4096,
        }
    }

    fn msg(role: &str, content: String) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content,
            name: None,
            images: None,
        }
    }

    async fn complete(&self, system: &str, user: String) -> Result<(String, u64), GeneratorError> {
        let messages = vec![
            Self::msg("system", system.to_string()),
            Self::msg("user", user),
        ];
        let resp = self
            .backend
            .chat(&messages, None, self.temperature, self.max_tokens)
            .await
            .map_err(|e| GeneratorError::Llm(e.to_string()))?;
        let tokens = resp
            .usage
            .as_ref()
            .map(|u| u.total_tokens as u64)
            .unwrap_or((resp.content.len() / 4) as u64);
        Ok((resp.content, tokens))
    }
}

/// Extract the first top-level JSON object from an LLM response (tolerates code fences).
fn extract_json(s: &str) -> Result<serde_json::Value, GeneratorError> {
    let trimmed = s.trim();
    let cleaned = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim_end_matches("```")
        .trim();
    // Find the outermost braces if there is surrounding prose.
    let start = cleaned.find('{');
    let end = cleaned.rfind('}');
    let slice = match (start, end) {
        (Some(a), Some(b)) if b > a => &cleaned[a..=b],
        _ => cleaned,
    };
    serde_json::from_str(slice).map_err(|e| GeneratorError::Parse(e.to_string()))
}

#[async_trait]
impl SkillGenerator for LlmSkillGenerator {
    async fn extract_requirements(
        &self,
        prompt: &str,
    ) -> Result<(SkillRequirement, u64), GeneratorError> {
        let system = "You are KRIA's skill requirement analyst. Given a user goal, output ONLY a JSON object with fields: intent (string), category (string), tags (string[]), constraints (string[]), implied_capabilities (string[] from: filesystem_read, filesystem_write, filesystem_delete, network_egress, subprocess, browser, gpu, environment_secrets), dependencies (string[]), failure_cases (string[]), edge_cases (string[]), confidence (0..1).";
        let (content, tokens) = self.complete(system, format!("Goal: {prompt}")).await?;
        let v = extract_json(&content)?;

        let arr = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        let req = SkillRequirement {
            intent: v
                .get("intent")
                .and_then(|x| x.as_str())
                .unwrap_or(prompt)
                .to_string(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            constraints: arr("constraints"),
            implied_capabilities: arr("implied_capabilities"),
            category: v
                .get("category")
                .and_then(|x| x.as_str())
                .unwrap_or("misc")
                .to_string(),
            tags: arr("tags"),
            dependencies: arr("dependencies"),
            failure_cases: arr("failure_cases"),
            edge_cases: arr("edge_cases"),
            confidence: v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.5),
        };
        Ok((req, tokens))
    }

    async fn design_skill(
        &self,
        req: &SkillRequirement,
    ) -> Result<(SkillDesign, u64), GeneratorError> {
        let system = "You are KRIA's skill designer. Output ONLY JSON: name, slug (oc_snake_case), description (<=100 chars), version (semver), schema (JSON Schema object), examples ([{description, params}]), documentation (markdown), resource_class (light|medium|heavy).";
        let user = serde_json::to_string(req).unwrap_or_default();
        let (content, tokens) = self
            .complete(system, format!("Requirement: {user}"))
            .await?;
        let v = extract_json(&content)?;

        // Capabilities + risk are inferred by KRIA (never trusted from the LLM) — A9.4.
        let capabilities = infer_capabilities(req);
        let risk = classify_risk(&capabilities);

        let examples = v
            .get("examples")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        Some(SkillExample {
                            description: e.get("description")?.as_str()?.to_string(),
                            params: e.get("params").cloned().unwrap_or(serde_json::json!({})),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let slug = v
            .get("slug")
            .and_then(|x| x.as_str())
            .map(sanitize_slug)
            .unwrap_or_else(|| sanitize_slug(&req.intent));

        let design = SkillDesign {
            name: v
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("Generated Skill")
                .to_string(),
            slug,
            description: v
                .get("description")
                .and_then(|x| x.as_str())
                .unwrap_or(&req.intent)
                .chars()
                .take(100)
                .collect(),
            category: req.category.clone(),
            tags: req.tags.clone(),
            version: v
                .get("version")
                .and_then(|x| x.as_str())
                .unwrap_or("1.0.0")
                .to_string(),
            capabilities,
            dependencies: req.dependencies.clone(),
            risk,
            schema: v
                .get("schema")
                .cloned()
                .unwrap_or(serde_json::json!({"type":"object","properties":{}})),
            examples,
            documentation: v
                .get("documentation")
                .and_then(|x| x.as_str())
                .unwrap_or("Generated by KRIA ASGS.")
                .to_string(),
            runtime_kind: "docker".to_string(),
            entry: "handler/main.js".to_string(),
            resource_class: v
                .get("resource_class")
                .and_then(|x| x.as_str())
                .unwrap_or("light")
                .to_string(),
        };
        Ok((design, tokens))
    }

    async fn generate_code(
        &self,
        design: &SkillDesign,
    ) -> Result<(GeneratedArtifacts, u64), GeneratorError> {
        let system = "You are KRIA's production code generator. Output ONLY JSON: handler_code (complete Node.js module.exports async handler with error handling and logging, NO TODOs/placeholders), test_code (a test file), examples_doc (markdown).";
        let user = format!(
            "Design: name={}, description={}, schema={}",
            design.name, design.description, design.schema
        );
        let (content, tokens) = self.complete(system, user).await?;
        let v = extract_json(&content)?;
        let artifacts = GeneratedArtifacts {
            handler_code: v
                .get("handler_code")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            test_code: v
                .get("test_code")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            examples_doc: v
                .get("examples_doc")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        };
        Ok((artifacts, tokens))
    }

    async fn repair_code(
        &self,
        design: &SkillDesign,
        current: &GeneratedArtifacts,
        failure: &str,
    ) -> Result<(GeneratedArtifacts, u64), GeneratorError> {
        let system = "You are KRIA's autonomous repair engine. Given a failing skill and the failure, output ONLY JSON: handler_code, test_code, examples_doc — fully fixed, no placeholders.";
        let user = format!(
            "Design: {}\nFailure: {}\nCurrent handler:\n{}",
            design.name, failure, current.handler_code
        );
        let (content, tokens) = self.complete(system, user).await?;
        let v = extract_json(&content)?;
        let artifacts = GeneratedArtifacts {
            handler_code: v
                .get("handler_code")
                .and_then(|x| x.as_str())
                .unwrap_or(&current.handler_code)
                .to_string(),
            test_code: v
                .get("test_code")
                .and_then(|x| x.as_str())
                .unwrap_or(&current.test_code)
                .to_string(),
            examples_doc: v
                .get("examples_doc")
                .and_then(|x| x.as_str())
                .unwrap_or(&current.examples_doc)
                .to_string(),
        };
        Ok((artifacts, tokens))
    }
}

/// Normalize a slug to `oc_snake_case` satisfying the manifest slug rule.
fn sanitize_slug(s: &str) -> String {
    let mut out: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    let out = out.trim_matches('_').to_string();
    let out = if out.starts_with("oc_") {
        out
    } else {
        format!("oc_{out}")
    };
    out.chars().take(64).collect()
}
