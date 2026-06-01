use super::types::N8nWorkflowConfig;
use crate::safety::RiskLevel;
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeSet;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct N8nWorkflowMatchCandidate {
    pub workflow_id: String,
    pub display_name: String,
    pub status: String,
    pub matched_on: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum N8nWorkflowReferenceMatch<'a> {
    Unique {
        workflow: &'a N8nWorkflowConfig,
        matched_on: Vec<String>,
    },
    Ambiguous {
        matches: Vec<N8nWorkflowMatchCandidate>,
    },
    NoMatch {
        available: Vec<N8nWorkflowMatchCandidate>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct WorkflowCandidate {
    pub workflow_id: String,
    pub workflow_version: String,
    pub display_name: String,
    pub category: String,
    pub risk_tier: String,
    pub status: String,
    pub hitl_policy: String,
    pub score: f32,
    pub confidence: f32,
    pub confidence_label: String,
    pub matched_on: Vec<String>,
    pub requires_confirmation: bool,
    pub suggested_input_payload: Value,
    pub missing_inputs: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct WorkflowSuggestionResponse {
    pub schema_version: String,
    pub prompt: String,
    pub reference: String,
    pub status: String,
    pub candidates: Vec<WorkflowCandidate>,
    pub requires_confirmation: bool,
    pub can_auto_run: bool,
    pub ambiguous: bool,
    pub hard_prompt: bool,
    pub message: String,
    pub confirmation_hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowRankingEngine {
    workflows: Vec<N8nWorkflowConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetadataField {
    WorkflowId,
    DisplayName,
    Alias,
    ExamplePrompt,
    Tag,
    Category,
}

impl MetadataField {
    fn label(self) -> &'static str {
        match self {
            Self::WorkflowId => "workflow_id",
            Self::DisplayName => "display_name",
            Self::Alias => "alias",
            Self::ExamplePrompt => "example_prompt",
            Self::Tag => "tag",
            Self::Category => "category",
        }
    }

    fn exact_score(self) -> f32 {
        match self {
            Self::WorkflowId => 100.0,
            Self::DisplayName => 96.0,
            Self::Alias => 92.0,
            Self::ExamplePrompt => 88.0,
            Self::Tag => 74.0,
            Self::Category => 62.0,
        }
    }

    fn contains_score(self) -> f32 {
        match self {
            Self::WorkflowId => 86.0,
            Self::DisplayName => 82.0,
            Self::Alias => 78.0,
            Self::ExamplePrompt => 76.0,
            Self::Tag => 58.0,
            Self::Category => 48.0,
        }
    }
}

#[derive(Debug, Clone)]
struct CandidateScore {
    workflow: N8nWorkflowConfig,
    score: f32,
    matched_on: BTreeSet<String>,
    reason: String,
}

fn normalize_reference(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn normalize_for_tokens(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(' ');
        }
    }
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn token_set(value: &str) -> BTreeSet<String> {
    const STOPWORDS: &[&str] = &[
        "a", "an", "and", "are", "at", "by", "for", "from", "i", "in", "is", "it", "me", "my",
        "of", "on", "or", "please", "the", "this", "to", "with",
    ];
    normalize_for_tokens(value)
        .split_whitespace()
        .filter(|token| token.len() > 1 && !STOPWORDS.contains(token))
        .map(str::to_string)
        .collect()
}

fn broad_prompt(value: &str) -> bool {
    let normalized = normalize_reference(value);
    let hard_phrases = [
        "brief me",
        "check if automation is healthy",
        "clean up",
        "discuss the report",
        "find out what everyone sent",
        "get the report from mail",
        "handle my email",
        "handle the client follow-up",
        "handle this payment thing",
        "organize bug reports",
        "process this document",
        "publish the update",
        "reply to everyone",
        "send the report to the team",
        "share the report with everyone",
        "summarize everything",
        "test everything",
        "track this bug",
    ];

    hard_phrases
        .iter()
        .any(|phrase| normalized == *phrase || normalized.contains(phrase))
}

fn candidate(workflow: &N8nWorkflowConfig, matched_on: Vec<String>) -> N8nWorkflowMatchCandidate {
    N8nWorkflowMatchCandidate {
        workflow_id: workflow.workflow_id.clone(),
        display_name: if workflow.display_name.trim().is_empty() {
            workflow.workflow_id.clone()
        } else {
            workflow.display_name.clone()
        },
        status: format!("{:?}", workflow.status).to_ascii_lowercase(),
        matched_on,
    }
}

fn workflow_match_keys(workflow: &N8nWorkflowConfig) -> Vec<(String, String)> {
    let mut keys = vec![
        ("workflow_id".to_string(), workflow.workflow_id.clone()),
        ("display_name".to_string(), workflow.display_name.clone()),
        ("category".to_string(), workflow.category.clone()),
    ];

    keys.extend(
        workflow
            .aliases
            .iter()
            .cloned()
            .map(|alias| ("alias".to_string(), alias)),
    );
    keys.extend(
        workflow
            .tags
            .iter()
            .cloned()
            .map(|tag| ("tag".to_string(), tag)),
    );

    keys
}

fn ranking_keys(workflow: &N8nWorkflowConfig) -> Vec<(MetadataField, String)> {
    let mut keys = vec![
        (MetadataField::WorkflowId, workflow.workflow_id.clone()),
        (MetadataField::DisplayName, workflow.display_name.clone()),
        (MetadataField::Category, workflow.category.clone()),
    ];
    keys.extend(
        workflow
            .aliases
            .iter()
            .cloned()
            .map(|value| (MetadataField::Alias, value)),
    );
    keys.extend(
        workflow
            .tags
            .iter()
            .cloned()
            .map(|value| (MetadataField::Tag, value)),
    );
    keys.extend(
        workflow
            .example_prompts
            .iter()
            .cloned()
            .map(|value| (MetadataField::ExamplePrompt, value)),
    );
    keys
}

fn confidence_label(confidence: f32) -> String {
    if confidence >= 0.90 {
        "high".into()
    } else if confidence >= 0.70 {
        "medium".into()
    } else {
        "low".into()
    }
}

fn candidate_from_score(score: CandidateScore, prompt: &str) -> WorkflowCandidate {
    let workflow = score.workflow;
    let confidence = (score.score / 100.0).clamp(0.0, 1.0);
    let suggested_input_payload = build_n8n_suggested_input_payload(&workflow, prompt, false);
    let missing_inputs =
        super::schema::input_payload_validation_issues(&workflow, &suggested_input_payload);
    WorkflowCandidate {
        workflow_id: workflow.workflow_id.clone(),
        workflow_version: workflow.workflow_version.clone(),
        display_name: if workflow.display_name.trim().is_empty() {
            workflow.workflow_id.clone()
        } else {
            workflow.display_name.clone()
        },
        category: workflow.category.clone(),
        risk_tier: format!("{:?}", workflow.risk_tier),
        status: format!("{:?}", workflow.status).to_ascii_lowercase(),
        hitl_policy: workflow.hitl_policy.clone(),
        score: (score.score * 10.0).round() / 10.0,
        confidence,
        confidence_label: confidence_label(confidence),
        matched_on: score.matched_on.into_iter().collect(),
        requires_confirmation: true,
        suggested_input_payload,
        missing_inputs,
        reason: score.reason,
    }
}

pub fn build_n8n_suggested_input_payload(
    workflow: &N8nWorkflowConfig,
    prompt: &str,
    confirmed: bool,
) -> Value {
    let mut payload = Map::new();
    insert_non_empty(&mut payload, "source_prompt", prompt.trim());

    match workflow.workflow_id.as_str() {
        "gmail_inbox_digest" => {
            if prompt.to_ascii_lowercase().contains("today") {
                insert_non_empty(&mut payload, "time_window", "today");
            }
            payload.insert("max_messages".into(), Value::from(10));
        }
        "gmail_search_messages" => {
            insert_non_empty(&mut payload, "query", prompt.trim());
            payload.insert("max_results".into(), Value::from(10));
        }
        "gmail_send_draft" => {
            if let Some(recipient) =
                extract_after_until(prompt, " to ", &[" about ", " saying ", " that ", ","])
            {
                insert_non_empty(&mut payload, "recipient", &recipient);
            }
            if let Some(subject) =
                extract_after_until(prompt, " about ", &[" saying ", " that ", "."])
            {
                insert_non_empty(&mut payload, "subject", &subject);
            }
            if let Some(body) = extract_quoted(prompt)
                .or_else(|| extract_after_until(prompt, " saying ", &["."]))
                .or_else(|| extract_after_until(prompt, " telling them ", &["."]))
                .or_else(|| extract_after_until(prompt, " that ", &["."]))
            {
                insert_non_empty(&mut payload, "body", &body);
            }
        }
        "calendar_create_meeting" => {
            insert_non_empty(&mut payload, "title", cleanup_sentence(prompt));
            if let Some(attendee) = extract_after_until(
                prompt,
                " with ",
                &[" tomorrow", " next week", " today", " for ", ",", "."],
            ) {
                let attendee = attendee.trim();
                if !attendee.is_empty() {
                    payload.insert(
                        "attendees".into(),
                        Value::Array(vec![Value::String(attendee.into())]),
                    );
                }
            }
            if let Some(start_time) = extract_calendar_time(prompt) {
                insert_non_empty(&mut payload, "start_time", &start_time);
            }
            if let Some(duration) = extract_duration_minutes(prompt) {
                payload.insert("duration_minutes".into(), Value::from(duration));
            }
        }
        "slack_post_update" => {
            if let Some(channel) = extract_slack_channel(prompt) {
                insert_non_empty(&mut payload, "channel", &channel);
            }
            if let Some(message) = extract_quoted(prompt)
                .or_else(|| {
                    extract_after_until(
                        prompt,
                        "let the team know ",
                        &[" to slack", " in slack", " to #", " in #", "."],
                    )
                })
                .or_else(|| {
                    extract_after_until(
                        prompt,
                        "announce ",
                        &[" to slack", " in slack", " to #", " in #", "."],
                    )
                })
                .or_else(|| {
                    extract_after_until(
                        prompt,
                        "publish ",
                        &[" to slack", " in slack", " to #", " in #", "."],
                    )
                })
                .or_else(|| {
                    extract_after_until(
                        prompt,
                        "share ",
                        &[" to slack", " in slack", " to #", " in #", "."],
                    )
                })
            {
                insert_non_empty(&mut payload, "message", &message);
            }
        }
        _ => {}
    }

    if WorkflowConfirmationFlow::workflow_requires_confirmation(workflow) {
        payload.insert("confirmed_by_user".into(), Value::Bool(confirmed));
    }

    Value::Object(payload)
}

pub fn mark_n8n_input_payload_confirmed(
    workflow: &N8nWorkflowConfig,
    input_payload: Value,
) -> Value {
    if !WorkflowConfirmationFlow::workflow_requires_confirmation(workflow) {
        return input_payload;
    }

    match input_payload {
        Value::Object(mut map) => {
            map.insert("confirmed_by_user".into(), Value::Bool(true));
            Value::Object(map)
        }
        other => serde_json::json!({
            "source_payload": other,
            "confirmed_by_user": true,
        }),
    }
}

fn insert_non_empty(payload: &mut Map<String, Value>, key: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        payload.insert(key.into(), Value::String(value.to_string()));
    }
}

fn cleanup_sentence(value: &str) -> &str {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
}

fn extract_quoted(value: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let Some(start) = value.find(quote) else {
            continue;
        };
        let rest = &value[start + quote.len_utf8()..];
        let Some(end) = rest.find(quote) else {
            continue;
        };
        let extracted = rest[..end].trim();
        if !extracted.is_empty() {
            return Some(extracted.to_string());
        }
    }
    None
}

fn extract_after_until(value: &str, phrase: &str, stops: &[&str]) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let start = lower.find(phrase)? + phrase.len();
    let mut end = value.len();
    let lower_tail = &lower[start..];
    for stop in stops {
        if let Some(offset) = lower_tail.find(stop) {
            end = end.min(start + offset);
        }
    }
    let extracted = cleanup_sentence(&value[start..end]);
    if extracted.is_empty() {
        None
    } else {
        Some(extracted.to_string())
    }
}

fn extract_slack_channel(prompt: &str) -> Option<String> {
    for token in prompt.split_whitespace() {
        let cleaned =
            token.trim_matches(|ch: char| matches!(ch, ',' | '.' | ':' | ';' | '"' | '\'' | '`'));
        if cleaned.starts_with('#') && cleaned.len() > 1 {
            return Some(cleaned.to_string());
        }
    }
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("team channel") {
        return Some("team".into());
    }
    if lower.contains("project channel") {
        return Some("project".into());
    }
    None
}

fn extract_calendar_time(prompt: &str) -> Option<String> {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("tomorrow") {
        Some("tomorrow".into())
    } else if lower.contains("next week") {
        Some("next week".into())
    } else if lower.contains("today") {
        Some("today".into())
    } else {
        None
    }
}

fn extract_duration_minutes(prompt: &str) -> Option<i64> {
    let tokens = prompt
        .split_whitespace()
        .map(|token| token.trim_matches(|ch: char| matches!(ch, ',' | '.' | ':' | ';')))
        .collect::<Vec<_>>();
    for window in tokens.windows(2) {
        if window[1].to_ascii_lowercase().starts_with("minute") {
            if let Ok(value) = window[0].parse::<i64>() {
                return Some(value);
            }
        }
    }
    None
}

pub fn parse_n8n_workflow_run_reference(message: &str) -> Option<String> {
    let trimmed = message.trim();
    let lower = trimmed.to_ascii_lowercase();
    let prefixes = [
        "invoke n8n workflow ",
        "trigger n8n workflow ",
        "start n8n workflow ",
        "execute n8n workflow ",
        "run n8n workflow ",
        "invoke workflow ",
        "trigger workflow ",
        "start workflow ",
        "execute workflow ",
        "run workflow ",
        "retry n8n workflow ",
        "retry workflow ",
        "rerun n8n workflow ",
        "rerun workflow ",
        "re-run n8n workflow ",
        "re-run workflow ",
        "run ",
        "retry ",
        "rerun ",
        "re-run ",
    ];

    let prefix = prefixes.iter().find(|prefix| lower.starts_with(**prefix))?;
    let mut reference = trimmed[prefix.len()..].trim();
    if reference.to_ascii_lowercase().starts_with("the ") {
        reference = reference[4..].trim();
    }

    let mut cleaned = reference
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
        .trim()
        .to_string();

    for suffix in [", please", " please", ", now", " now", ", again", " again"] {
        let lower_cleaned = cleaned.to_ascii_lowercase();
        if lower_cleaned.ends_with(suffix) && lower_cleaned.len() > suffix.len() {
            let new_len = cleaned.len().saturating_sub(suffix.len());
            cleaned.truncate(new_len);
            cleaned = cleaned
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
                .trim()
                .to_string();
        }
    }

    if cleaned.is_empty() {
        return None;
    }
    if cleaned
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '/' | '\\'))
    {
        return None;
    }

    Some(cleaned)
}

impl WorkflowRankingEngine {
    pub fn new(workflows: Vec<N8nWorkflowConfig>) -> Self {
        Self { workflows }
    }

    pub fn suggest(&self, prompt: &str) -> WorkflowSuggestionResponse {
        let reference =
            parse_n8n_workflow_run_reference(prompt).unwrap_or_else(|| prompt.trim().to_string());
        self.suggest_for_reference(prompt, &reference)
    }

    pub fn suggest_for_reference(
        &self,
        prompt: &str,
        reference: &str,
    ) -> WorkflowSuggestionResponse {
        let normalized_reference = normalize_reference(reference);
        let reference_tokens = token_set(reference);
        let mut scored = Vec::new();

        for workflow in self
            .workflows
            .iter()
            .filter(|workflow| workflow.is_approved_for_execution())
        {
            let mut best_score = 0.0_f32;
            let mut matched_on = BTreeSet::new();
            let mut best_reason = String::new();

            for (field, value) in ranking_keys(workflow) {
                let key = normalize_reference(&value);
                if key.is_empty() || normalized_reference.is_empty() {
                    continue;
                }

                if key == normalized_reference {
                    let score = field.exact_score();
                    if score > best_score {
                        best_score = score;
                        best_reason = format!("Exact {} match", field.label());
                    }
                    matched_on.insert(field.label().to_string());
                    continue;
                }

                if key.len() >= 4
                    && normalized_reference.len() >= 4
                    && (key.contains(&normalized_reference) || normalized_reference.contains(&key))
                {
                    let score = field.contains_score();
                    if score > best_score {
                        best_score = score;
                        best_reason = format!("Phrase overlap with {}", field.label());
                    }
                    matched_on.insert(field.label().to_string());
                    continue;
                }

                let key_tokens = token_set(&key);
                if key_tokens.is_empty() || reference_tokens.is_empty() {
                    continue;
                }
                let overlap = key_tokens.intersection(&reference_tokens).count();
                let denominator = key_tokens.len().max(reference_tokens.len());
                let ratio = overlap as f32 / denominator as f32;
                if overlap >= 2 && ratio >= 0.45 {
                    let score = (field.contains_score() * ratio).max(44.0);
                    if score > best_score {
                        best_score = score;
                        best_reason = format!("Token overlap with {}", field.label());
                    }
                    matched_on.insert(field.label().to_string());
                }
            }

            if best_score >= 44.0 {
                scored.push(CandidateScore {
                    workflow: workflow.clone(),
                    score: best_score,
                    matched_on,
                    reason: best_reason,
                });
            }
        }

        scored.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.workflow.workflow_id.cmp(&right.workflow.workflow_id))
        });

        let hard_prompt = broad_prompt(reference);
        let ambiguous = scored.len() > 1
            && scored
                .first()
                .map(|top| {
                    scored
                        .iter()
                        .skip(1)
                        .any(|candidate| top.score - candidate.score <= 18.0)
                })
                .unwrap_or(false);
        let top_candidates = scored
            .into_iter()
            .take(3)
            .map(|score| candidate_from_score(score, prompt))
            .collect::<Vec<_>>();

        let (status, message, confirmation_hint) = if top_candidates.is_empty() && hard_prompt {
            (
                "needs_clarification".to_string(),
                format!("I need more detail before choosing an n8n workflow for \"{reference}\"."),
                None,
            )
        } else if top_candidates.is_empty() {
            (
                "not_found".to_string(),
                format!("Workflow \"{reference}\" was not found in approved n8n workflows."),
                None,
            )
        } else if hard_prompt || ambiguous {
            (
                "needs_clarification".to_string(),
                format!(
                    "I found {} possible n8n workflow(s). Choose one before I run anything.",
                    top_candidates.len()
                ),
                Some(format!(
                    "Confirm with: Confirm workflow {}",
                    top_candidates[0].workflow_id
                )),
            )
        } else {
            (
                "needs_confirmation".to_string(),
                format!(
                    "I found \"{}\". Confirm before I run it.",
                    top_candidates[0].display_name
                ),
                Some(format!(
                    "Confirm with: Confirm workflow {}",
                    top_candidates[0].workflow_id
                )),
            )
        };

        WorkflowSuggestionResponse {
            schema_version: "kria.n8n.workflow_suggestion.v1".into(),
            prompt: prompt.to_string(),
            reference: reference.to_string(),
            status,
            candidates: top_candidates,
            requires_confirmation: true,
            can_auto_run: false,
            ambiguous,
            hard_prompt,
            message,
            confirmation_hint,
        }
    }
}

pub struct WorkflowConfirmationFlow;

impl WorkflowConfirmationFlow {
    pub fn parse_confirmation_reference(message: &str) -> Option<String> {
        let trimmed = message.trim();
        let lower = trimmed.to_ascii_lowercase();
        let prefixes = [
            "confirm n8n workflow ",
            "confirm workflow ",
            "confirm and run n8n workflow ",
            "confirm and run workflow ",
            "run confirmed n8n workflow ",
            "run confirmed workflow ",
            "execute confirmed n8n workflow ",
            "execute confirmed workflow ",
            "yes run n8n workflow ",
            "yes run workflow ",
            "yes run ",
        ];
        let prefix = prefixes.iter().find(|prefix| lower.starts_with(**prefix))?;
        let mut reference = trimmed[prefix.len()..]
            .trim()
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
            .to_string();
        for suffix in [", please", " please", ", now", " now"] {
            let lower_reference = reference.to_ascii_lowercase();
            if lower_reference.ends_with(suffix) && lower_reference.len() > suffix.len() {
                let new_len = reference.len().saturating_sub(suffix.len());
                reference.truncate(new_len);
                reference = reference
                    .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | '.' | ':' | ';'))
                    .trim()
                    .to_string();
            }
        }
        if reference.is_empty()
            || reference
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '/' | '\\'))
        {
            return None;
        }
        Some(reference)
    }

    pub fn workflow_requires_confirmation(workflow: &N8nWorkflowConfig) -> bool {
        !matches!(workflow.risk_tier, RiskLevel::Green)
            || !workflow.hitl_policy.trim().eq_ignore_ascii_case("none")
    }

    pub fn candidate_confirmation_text(candidate: &WorkflowCandidate) -> String {
        format!("Confirm workflow {}", candidate.workflow_id)
    }
}

pub fn resolve_n8n_workflow_reference<'a>(
    workflows: &'a [N8nWorkflowConfig],
    reference: &str,
) -> N8nWorkflowReferenceMatch<'a> {
    let normalized = normalize_reference(reference);
    if normalized.is_empty() {
        return N8nWorkflowReferenceMatch::NoMatch {
            available: workflows
                .iter()
                .map(|workflow| candidate(workflow, Vec::new()))
                .collect(),
        };
    }

    let mut matches: Vec<(&N8nWorkflowConfig, Vec<String>)> = Vec::new();
    for workflow in workflows {
        let mut matched_on = Vec::new();
        for (source, key) in workflow_match_keys(workflow) {
            let key = normalize_reference(&key);
            if !key.is_empty() && key == normalized {
                matched_on.push(source);
            }
        }
        if !matched_on.is_empty() {
            matches.push((workflow, matched_on));
        }
    }

    if matches.len() == 1 {
        let (workflow, matched_on) = matches.remove(0);
        return N8nWorkflowReferenceMatch::Unique {
            workflow,
            matched_on,
        };
    }

    if matches.len() > 1 {
        return N8nWorkflowReferenceMatch::Ambiguous {
            matches: matches
                .into_iter()
                .map(|(workflow, matched_on)| candidate(workflow, matched_on))
                .collect(),
        };
    }

    N8nWorkflowReferenceMatch::NoMatch {
        available: workflows
            .iter()
            .map(|workflow| candidate(workflow, Vec::new()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::n8n::{N8nWorkflowConfig, N8nWorkflowStatus};

    fn workflow(id: &str, display_name: &str) -> N8nWorkflowConfig {
        N8nWorkflowConfig {
            workflow_id: id.into(),
            workflow_version: "v1".into(),
            display_name: display_name.into(),
            endpoint_path: format!("/webhook/{id}"),
            status: N8nWorkflowStatus::Approved,
            category: "diagnostic".into(),
            aliases: vec![format!("{display_name} alias")],
            tags: vec!["diagnostic".into()],
            ..Default::default()
        }
    }

    #[test]
    fn parses_full_workflow_references_without_first_token_truncation() {
        assert_eq!(
            parse_n8n_workflow_run_reference("Run Test Workflow"),
            Some("Test Workflow".into())
        );
        assert_eq!(
            parse_n8n_workflow_run_reference("run workflow `demo_flow`, please"),
            Some("demo_flow".into())
        );
        assert_eq!(
            parse_n8n_workflow_run_reference("trigger n8n workflow invoice.sync-v1 now"),
            Some("invoice.sync-v1".into())
        );
        assert_eq!(parse_n8n_workflow_run_reference("run ../../secret"), None);
    }

    #[test]
    fn resolves_exact_id_display_name_alias_and_tag() {
        let workflows = vec![workflow("test_workflow", "Test Workflow")];

        assert!(matches!(
            resolve_n8n_workflow_reference(&workflows, "test_workflow"),
            N8nWorkflowReferenceMatch::Unique { .. }
        ));
        assert!(matches!(
            resolve_n8n_workflow_reference(&workflows, "Test Workflow"),
            N8nWorkflowReferenceMatch::Unique { .. }
        ));
        assert!(matches!(
            resolve_n8n_workflow_reference(&workflows, "Test Workflow Alias"),
            N8nWorkflowReferenceMatch::Unique { .. }
        ));
        assert!(matches!(
            resolve_n8n_workflow_reference(&workflows, "diagnostic"),
            N8nWorkflowReferenceMatch::Unique { .. }
        ));
    }

    #[test]
    fn returns_ambiguity_for_exact_multi_workflow_matches() {
        let workflows = vec![
            workflow("test_workflow", "Test Workflow"),
            workflow("other_workflow", "Other Workflow"),
        ];

        let result = resolve_n8n_workflow_reference(&workflows, "diagnostic");

        match result {
            N8nWorkflowReferenceMatch::Ambiguous { matches } => assert_eq!(matches.len(), 2),
            other => panic!("expected ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn returns_available_workflows_for_no_match() {
        let workflows = vec![workflow("test_workflow", "Test Workflow")];

        let result = resolve_n8n_workflow_reference(&workflows, "missing");

        match result {
            N8nWorkflowReferenceMatch::NoMatch { available } => {
                assert_eq!(available[0].workflow_id, "test_workflow")
            }
            other => panic!("expected no match, got {other:?}"),
        }
    }

    #[test]
    fn ranks_candidates_from_metadata_without_auto_run() {
        let mut inbox = workflow("gmail_inbox_digest", "Inbox Digest");
        inbox.category = "email".into();
        inbox.aliases = vec!["summarize my inbox".into()];
        inbox.example_prompts = vec!["what did i miss in email this morning".into()];
        inbox.tags = vec!["email".into(), "inbox".into(), "digest".into()];

        let mut search = workflow("gmail_search_messages", "Gmail Message Search");
        search.category = "email".into();
        search.aliases = vec!["search gmail messages".into()];
        search.tags = vec!["email".into(), "search".into()];

        let engine = WorkflowRankingEngine::new(vec![inbox, search]);
        let response = engine.suggest("What did I miss in email this morning");

        assert_eq!(response.status, "needs_confirmation");
        assert!(!response.can_auto_run);
        assert_eq!(response.candidates[0].workflow_id, "gmail_inbox_digest");
        assert!(response.candidates[0]
            .matched_on
            .contains(&"example_prompt".to_string()));
    }

    #[test]
    fn hard_or_ambiguous_prompts_return_top_candidates_without_auto_run() {
        let mut draft = workflow("gmail_send_draft", "Gmail Draft Creator");
        draft.category = "email".into();
        draft.aliases = vec!["share the report with everyone".into()];
        draft.risk_tier = RiskLevel::Yellow;
        draft.hitl_policy = "required_review".into();

        let mut slack = workflow("slack_post_update", "Slack Update Poster");
        slack.category = "messaging".into();
        slack.aliases = vec!["share the report with everyone".into()];
        slack.risk_tier = RiskLevel::Yellow;
        slack.hitl_policy = "required_review".into();

        let engine = WorkflowRankingEngine::new(vec![draft, slack]);
        let response = engine.suggest("Share the report with everyone");

        assert_eq!(response.status, "needs_clarification");
        assert!(response.hard_prompt);
        assert!(!response.can_auto_run);
        assert_eq!(response.candidates.len(), 2);
    }

    #[test]
    fn parses_confirmation_references() {
        assert_eq!(
            WorkflowConfirmationFlow::parse_confirmation_reference(
                "Confirm workflow gmail_inbox_digest"
            ),
            Some("gmail_inbox_digest".into())
        );
        assert_eq!(
            WorkflowConfirmationFlow::parse_confirmation_reference(
                "yes run workflow `slack_post_update`, please"
            ),
            Some("slack_post_update".into())
        );
        assert_eq!(
            WorkflowConfirmationFlow::parse_confirmation_reference("confirm workflow ../../secret"),
            None
        );
    }
}
