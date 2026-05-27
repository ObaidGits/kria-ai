use super::*;

pub(super) fn is_gmail_tool_name(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "gw_gmail_inbox" | "gw_gmail_search" | "gw_gmail_read" | "gw_gmail_send"
    )
}

pub(super) fn looks_like_spurious_gmail_capability_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let unsupported = lower.contains("not directly supported")
        || lower.contains("not supported by the current interface")
        || lower.contains("use a web browser")
        || lower.contains("third-party application");
    let gmail_context =
        lower.contains("gmail") || lower.contains("email") || lower.contains("inbox");
    unsupported && gmail_context
}

pub(super) fn strip_spurious_gmail_error_lines(text: &str) -> String {
    let filtered = text
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            let lower = trimmed.to_ascii_lowercase();
            !lower.starts_with("tool_error:") && !looks_like_spurious_gmail_capability_line(trimmed)
        })
        .collect::<Vec<_>>()
        .join("\n");

    MULTI_NEWLINE_RE
        .replace_all(filtered.trim(), "\n\n")
        .to_string()
}

pub(super) fn extract_grounded_gmail_counts(tool_result: &serde_json::Value) -> Option<(u64, u64)> {
    let payload = tool_result.get("data").unwrap_or(tool_result);

    let requested = payload.get("requested_count").and_then(|v| v.as_u64());
    let returned = payload
        .get("returned_count")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            payload
                .get("messages")
                .and_then(|v| v.as_array())
                .map(|messages| messages.len() as u64)
        });

    match (requested, returned) {
        (Some(req), Some(ret)) => Some((req, ret)),
        (None, Some(ret)) => Some((ret, ret)),
        _ => None,
    }
}

/// Build a user-facing confirmation response for a successful `generate_image` call.
/// This avoids a second LLM round-trip that would crash ctx=2048 with 167 tool schemas.
pub(super) fn build_image_success_response(tool_result: &serde_json::Value) -> String {
    let images = tool_result.get("images").and_then(|v| v.as_array());
    let count = images.map(|a| a.len()).unwrap_or(0);
    let first = images.and_then(|a| a.first());
    let provenance = first
        .and_then(|img| img.get("provenance"))
        .and_then(|v| v.as_str())
        .unwrap_or("AI");
    let elapsed_ms = tool_result
        .get("elapsed_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let elapsed_s = elapsed_ms as f64 / 1000.0;
    let path = first
        .and_then(|img| img.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let seed = first
        .and_then(|img| img.get("seed"))
        .and_then(|v| v.as_u64());
    let quality = first
        .and_then(|img| img.get("quality"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let tier = tool_result
        .get("tier_used")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let source = if provenance.contains("pollinations") {
        "Pollinations.ai (Flux.1-schnell)"
    } else if provenance.starts_with("cloud") {
        "cloud AI"
    } else {
        "local AI"
    };

    let meta = {
        let mut parts = Vec::new();
        if !quality.is_empty() {
            parts.push(format!("quality: {quality}"));
        }
        if !tier.is_empty() {
            parts.push(format!("tier: {tier}"));
        }
        if let Some(s) = seed {
            parts.push(format!("seed: {s}"));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!(" ({})", parts.join(", "))
        }
    };

    if count == 1 && !path.is_empty() {
        format!("Image generated in {elapsed_s:.1}s using {source}{meta}.\nSaved to: `{path}`")
    } else if count > 1 {
        format!("{count} images generated in {elapsed_s:.1}s using {source}{meta}.")
    } else {
        "Image generated successfully.".to_string()
    }
}

pub(super) fn build_image_failure_response(data: &serde_json::Value) -> String {
    let report = data.get("failure_report");
    let stage = report
        .and_then(|r| r.get("stage"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let message = report
        .and_then(|r| r.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("Image generation failed");
    let hint = report
        .and_then(|r| r.get("hint"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let provider = report
        .and_then(|r| r.get("provider"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut msg = format!("Image generation failed at stage **{stage}**");
    if !provider.is_empty() {
        msg.push_str(&format!(" (provider: {provider})"));
    }
    msg.push_str(&format!(": {message}"));
    if !hint.is_empty() {
        msg.push_str(&format!("\n\nHint: {hint}"));
    }
    msg
}

pub(super) fn build_grounded_gmail_count_summary(
    tool_result: &serde_json::Value,
) -> Option<String> {
    let (requested, returned) = extract_grounded_gmail_counts(tool_result)?;

    if requested == returned {
        Some(format!("I retrieved {returned} grounded Gmail message(s)."))
    } else {
        Some(format!(
            "I retrieved {returned} grounded Gmail message(s) out of {requested} requested."
        ))
    }
}

pub(super) fn contains_gmail_placeholder_scaffold(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let has_bracket_placeholders = [
        "[sender's name]",
        "[sender’s name]",
        "[subject of the email]",
        "[preview of the email]",
        "[subject]",
        "[preview]",
    ]
    .iter()
    .any(|marker| lower.contains(marker));

    if has_bracket_placeholders {
        return true;
    }

    [
        "the exact content of the second",
        "the exact content of the third",
        "is not provided in the available data",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

pub(super) fn extract_prefixed_value_case_insensitive<'a>(
    line: &'a str,
    key: &str,
) -> Option<&'a str> {
    let trimmed = line.trim();
    let (prefix, value) = trimmed.split_once(':')?;
    if !prefix.trim().eq_ignore_ascii_case(key) {
        return None;
    }

    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub(super) fn normalize_gmail_row_value_for_dedup(value: &str) -> String {
    compact_text_for_llm(value, LLM_GMAIL_FIELD_MAX_CHARS).to_ascii_lowercase()
}

pub(super) fn contains_duplicate_gmail_rows(text: &str) -> bool {
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut id_occurrences = 0usize;
    let mut duplicate_ids = 0usize;

    let mut seen_from_subject_pairs: HashSet<String> = HashSet::new();
    let mut duplicate_pairs = 0usize;
    let mut pending_from: Option<String> = None;
    let mut pending_subject: Option<String> = None;

    for line in text.lines() {
        if let Some(id) = extract_prefixed_value_case_insensitive(line, "id") {
            let normalized_id = normalize_gmail_row_value_for_dedup(id);
            if !normalized_id.is_empty() {
                id_occurrences += 1;
                if !seen_ids.insert(normalized_id) {
                    duplicate_ids += 1;
                }
            }
        }

        if let Some(from) = extract_prefixed_value_case_insensitive(line, "from") {
            pending_from = Some(normalize_gmail_row_value_for_dedup(from));
        }

        if let Some(subject) = extract_prefixed_value_case_insensitive(line, "subject") {
            pending_subject = Some(normalize_gmail_row_value_for_dedup(subject));
        }

        if let (Some(from), Some(subject)) = (pending_from.as_ref(), pending_subject.as_ref()) {
            let signature = format!("{from}|{subject}");
            if !seen_from_subject_pairs.insert(signature) {
                duplicate_pairs += 1;
            }
            pending_from = None;
            pending_subject = None;
        }
    }

    (id_occurrences >= 2 && duplicate_ids > 0) || duplicate_pairs > 0
}

pub(super) fn dedupe_grounded_gmail_messages(
    messages: &[serde_json::Value],
) -> Vec<&serde_json::Value> {
    let mut deduped: Vec<&serde_json::Value> = Vec::with_capacity(messages.len());
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut seen_from_subject_pairs: HashSet<String> = HashSet::new();

    for message in messages {
        if let Some(id) = first_non_empty_string_field(
            message,
            &["id", "messageId", "message_id", "threadId", "thread_id"],
            LLM_GMAIL_FIELD_MAX_CHARS,
        ) {
            let key = normalize_gmail_row_value_for_dedup(&id);
            if key.is_empty() || seen_ids.insert(key) {
                deduped.push(message);
            }
            continue;
        }

        let from = first_non_empty_string_field(
            message,
            &["from", "sender", "organizer"],
            LLM_GMAIL_FIELD_MAX_CHARS,
        )
        .unwrap_or_default();

        let subject = first_non_empty_string_field(
            message,
            &["subject", "title", "summary"],
            LLM_GMAIL_FIELD_MAX_CHARS,
        )
        .unwrap_or_default();

        let signature = format!(
            "{}|{}",
            normalize_gmail_row_value_for_dedup(&from),
            normalize_gmail_row_value_for_dedup(&subject)
        );

        if signature == "|" || seen_from_subject_pairs.insert(signature) {
            deduped.push(message);
        }
    }

    deduped
}

pub(super) fn build_grounded_gmail_message_list_summary(
    tool_result: &serde_json::Value,
) -> Option<String> {
    let payload = tool_result.get("data").unwrap_or(tool_result);
    let messages = payload
        .get("messages")
        .or_else(|| payload.get("results"))
        .and_then(|v| v.as_array())?;

    if messages.is_empty() {
        return build_grounded_gmail_count_summary(tool_result);
    }

    let deduped_messages = dedupe_grounded_gmail_messages(messages);
    if deduped_messages.is_empty() {
        return build_grounded_gmail_count_summary(tool_result);
    }

    let (requested, returned) = extract_grounded_gmail_counts(tool_result)
        .unwrap_or((deduped_messages.len() as u64, deduped_messages.len() as u64));

    let returned_for_display = returned.min(deduped_messages.len() as u64);
    let visible_count = returned_for_display as usize;
    let mut lines = Vec::with_capacity(1 + visible_count * 3);

    if requested == returned_for_display {
        lines.push(format!(
            "I retrieved {returned_for_display} grounded Gmail message(s):"
        ));
    } else {
        lines.push(format!(
            "I retrieved {returned_for_display} grounded Gmail message(s) out of {requested} requested:"
        ));
    }

    for (index, message) in deduped_messages.iter().take(visible_count).enumerate() {
        let from = first_non_empty_string_field(
            message,
            &["from", "sender", "organizer"],
            LLM_GMAIL_FIELD_MAX_CHARS,
        )
        .unwrap_or_else(|| "Unknown sender".to_string());

        let subject = first_non_empty_string_field(
            message,
            &["subject", "title", "summary"],
            LLM_GMAIL_FIELD_MAX_CHARS,
        )
        .unwrap_or_else(|| "(No subject)".to_string());

        let preview = first_non_empty_string_field(
            message,
            &[
                "preview",
                "snippet",
                "description",
                "text",
                "content",
                "body",
            ],
            LLM_GMAIL_PREVIEW_MAX_CHARS,
        )
        .unwrap_or_else(|| "No preview available.".to_string());

        lines.push(format!("{}. From: {}", index + 1, from));
        lines.push(format!("   Subject: {}", subject));
        lines.push(format!("   Preview: {}", preview));
    }

    Some(lines.join("\n"))
}

pub(super) fn has_gmail_list_signal(text_lower: &str) -> bool {
    [
        "unread",
        "starred",
        "important",
        "sent",
        "draft",
        "spam",
        "trash",
    ]
    .iter()
    .any(|needle| text_lower.contains(needle))
}

pub(super) fn infer_gmail_search_query(user_text: &str) -> String {
    let lower = user_text.to_lowercase();
    for marker in [
        "search gmail for",
        "search my gmail for",
        "find in gmail",
        "find gmail for",
        "search email for",
        "search emails for",
    ] {
        if let Some((_, rest)) = lower.split_once(marker) {
            let query = rest.trim();
            if !query.is_empty() {
                return query.to_string();
            }
        }
    }

    if has_gmail_list_signal(&lower) {
        return infer_gmail_list_query(user_text);
    }

    user_text.trim().to_string()
}

pub(super) fn clean_gmail_body_candidate(candidate: &str) -> Option<String> {
    let mut body = candidate
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
        .trim()
        .to_string();

    if body.is_empty() {
        return None;
    }

    // Avoid turning vague references into accidental sends.
    let normalized = body.to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "mail" | "email" | "gmail" | "this" | "that" | "it"
    ) {
        return None;
    }

    // Strip trailing connective phrases that can leak from loose extraction.
    for marker in [" to ", " for "] {
        if let Some((head, _)) = body.split_once(marker) {
            let trimmed = head.trim();
            if !trimmed.is_empty() {
                body = trimmed.to_string();
            }
        }
    }

    if body.is_empty() {
        None
    } else {
        Some(body)
    }
}

pub(super) fn infer_gmail_send_body(user_text: &str) -> Option<String> {
    if let Some(caps) = GMAIL_SEND_BODY_AFTER_SAYING_RE.captures(user_text) {
        if let Some(matched) = caps.get(1) {
            if let Some(body) = clean_gmail_body_candidate(matched.as_str()) {
                return Some(body);
            }
        }
    }

    if let Some(caps) = GMAIL_SEND_BODY_BEFORE_MAIL_RE.captures(user_text) {
        if let Some(matched) = caps.get(1) {
            if let Some(body) = clean_gmail_body_candidate(matched.as_str()) {
                return Some(body);
            }
        }
    }

    if let Some(caps) = QUOTED_TEXT_RE.captures(user_text) {
        if let Some(matched) = caps.get(1).or_else(|| caps.get(2)) {
            if let Some(body) = clean_gmail_body_candidate(matched.as_str()) {
                return Some(body);
            }
        }
    }

    None
}

pub(super) fn infer_gmail_send_subject(user_text: &str, body: &str) -> String {
    if let Some(caps) = GMAIL_SEND_SUBJECT_RE.captures(user_text) {
        if let Some(matched) = caps.get(1) {
            let subject = matched.as_str().trim();
            if !subject.is_empty() {
                return subject.to_string();
            }
        }
    }

    let one_line = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if !one_line.is_empty() && one_line.len() <= 64 {
        return one_line;
    }

    "Message from KRIA".to_string()
}

pub(super) fn infer_gmail_send_arguments(user_text: &str) -> Option<serde_json::Value> {
    let to = CALENDAR_ATTENDEE_EMAIL_RE
        .captures(user_text)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_ascii_lowercase())?;

    let body = infer_gmail_send_body(user_text)?;
    let subject = infer_gmail_send_subject(user_text, &body);

    Some(serde_json::json!({
        "to": to,
        "subject": subject,
        "body": body,
    }))
}

pub(super) fn clean_identifier_candidate(candidate: &str) -> Option<String> {
    let id = candidate
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c == '(' || c == ')')
        .trim();

    if id.len() < 8 {
        return None;
    }

    if !id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '@'))
    {
        return None;
    }

    Some(id.to_string())
}

pub(super) fn clean_content_candidate(candidate: &str) -> Option<String> {
    let cleaned = candidate
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
        .trim()
        .trim_end_matches(['.', ',', ';', '!'])
        .trim();

    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

pub(super) fn extract_identifier_from_url_marker(text: &str, marker: &str) -> Option<String> {
    let (_, rest) = text.split_once(marker)?;
    let candidate = rest
        .trim_start()
        .split(|c: char| {
            c.is_whitespace() || matches!(c, '/' | '?' | '&' | '#' | ',' | ';' | '"' | '\'')
        })
        .next()
        .unwrap_or("");
    clean_identifier_candidate(candidate)
}

pub(super) fn extract_identifier_after_keyword(text: &str, keywords: &[&str]) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for keyword in keywords {
        if let Some(idx) = lower.find(keyword) {
            let start = idx + keyword.len();
            if let Some(rest) = text.get(start..) {
                let candidate = rest
                    .trim_start()
                    .trim_start_matches([':', '=', '#', '/'])
                    .split(|c: char| {
                        c.is_whitespace()
                            || matches!(
                                c,
                                '/' | '?' | '&' | '#' | ',' | ';' | '"' | '\'' | '(' | ')'
                            )
                    })
                    .next()
                    .unwrap_or("");
                if let Some(id) = clean_identifier_candidate(candidate) {
                    return Some(id);
                }
            }
        }
    }
    None
}

pub(super) fn infer_google_resource_id(user_text: &str) -> Option<String> {
    for marker in [
        "/document/d/",
        "/spreadsheets/d/",
        "/presentation/d/",
        "/file/d/",
        "/folders/",
        "id=",
    ] {
        if let Some(id) = extract_identifier_from_url_marker(user_text, marker) {
            return Some(id);
        }
    }

    if let Some(caps) = GENERIC_RESOURCE_ID_RE.captures(user_text) {
        if let Some(matched) = caps.get(1) {
            if let Some(id) = clean_identifier_candidate(matched.as_str()) {
                return Some(id);
            }
        }
    }

    if let Some(id) = extract_identifier_after_keyword(
        user_text,
        &[
            "file id",
            "file_id",
            "document id",
            "document_id",
            "spreadsheet id",
            "spreadsheet_id",
            "presentation id",
            "presentation_id",
            "id",
        ],
    ) {
        return Some(id);
    }

    if let Some(caps) = QUOTED_TEXT_RE.captures(user_text) {
        if let Some(matched) = caps.get(1).or_else(|| caps.get(2)) {
            let candidate = matched.as_str().trim();
            if candidate.len() >= 15 {
                return clean_identifier_candidate(candidate);
            }
        }
    }

    None
}

pub(super) fn infer_gmail_message_id(user_text: &str) -> Option<String> {
    if let Some(caps) = GMAIL_MESSAGE_ID_RE.captures(user_text) {
        if let Some(matched) = caps.get(1) {
            if let Some(id) = clean_identifier_candidate(matched.as_str()) {
                return Some(id);
            }
        }
    }

    for marker in ["/#inbox/", "/#all/", "/#sent/"] {
        if let Some(id) = extract_identifier_from_url_marker(user_text, marker) {
            return Some(id);
        }
    }

    let lower = user_text.to_ascii_lowercase();
    if lower.contains("gmail") || lower.contains("email") || lower.contains("mail") {
        return extract_identifier_after_keyword(user_text, &["message id", "message_id", "id"]);
    }

    None
}

pub(super) fn infer_calendar_event_id(user_text: &str) -> Option<String> {
    if let Some(caps) = CALENDAR_EVENT_ID_RE.captures(user_text) {
        if let Some(matched) = caps.get(1) {
            if let Some(id) = clean_identifier_candidate(matched.as_str()) {
                return Some(id);
            }
        }
    }

    let lower = user_text.to_ascii_lowercase();
    if lower.contains("calendar") || lower.contains("meeting") || lower.contains("event") {
        return extract_identifier_after_keyword(user_text, &["event id", "event_id", "id"]);
    }

    None
}

pub(super) fn infer_docs_edit_text(user_text: &str) -> Option<String> {
    if let Some(caps) = QUOTED_TEXT_RE.captures(user_text) {
        if let Some(matched) = caps.get(1).or_else(|| caps.get(2)) {
            if let Some(text) = clean_content_candidate(matched.as_str()) {
                return Some(text);
            }
        }
    }

    if let Some(caps) = APPEND_TEXT_RE.captures(user_text) {
        if let Some(matched) = caps.get(1) {
            if let Some(text) = clean_content_candidate(matched.as_str()) {
                return Some(text);
            }
        }
    }

    None
}

pub(super) fn infer_sheet_range(user_text: &str) -> Option<String> {
    let caps = SHEETS_RANGE_RE.captures(user_text)?;
    let matched = caps.get(1)?.as_str().trim();
    if matched.is_empty() {
        None
    } else {
        Some(matched.to_string())
    }
}

pub(super) fn infer_sheet_single_value(user_text: &str) -> Option<String> {
    if let Some(caps) = QUOTED_TEXT_RE.captures(user_text) {
        if let Some(matched) = caps.get(1).or_else(|| caps.get(2)) {
            return clean_content_candidate(matched.as_str());
        }
    }

    let lower = user_text.to_ascii_lowercase();
    for marker in [" to ", " value is ", " value ", " as "] {
        if let Some(idx) = lower.rfind(marker) {
            let start = idx + marker.len();
            if let Some(rest) = user_text.get(start..) {
                let candidate = rest
                    .trim_start()
                    .split(['\n', '\r', ',', ';', '!'])
                    .next()
                    .unwrap_or("")
                    .trim();
                if let Some(value) = clean_content_candidate(candidate) {
                    return Some(value);
                }
            }
        }
    }

    None
}

pub(super) fn looks_like_send_confirmation_prompt(user_text: &str) -> bool {
    SEND_CONFIRMATION_RE.is_match(user_text.trim())
}

pub(super) fn infer_confirmation_send_query_from_history(
    last_user_text: &str,
    messages: &[ChatMessage],
) -> Option<String> {
    if !looks_like_send_confirmation_prompt(last_user_text) {
        return None;
    }

    let mut to: Option<String> = None;
    let mut body: Option<String> = None;
    let mut subject: Option<String> = None;
    let mut skipped_current = false;

    for message in messages.iter().rev() {
        if message.role != "user" {
            continue;
        }

        if !skipped_current && message.content.trim() == last_user_text.trim() {
            skipped_current = true;
            continue;
        }

        if to.is_none() {
            to = CALENDAR_ATTENDEE_EMAIL_RE
                .captures(&message.content)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().trim().to_ascii_lowercase());
        }

        if body.is_none() {
            body = infer_gmail_send_body(&message.content);
        }

        if subject.is_none() {
            if let Some(existing_body) = body.as_deref() {
                subject = Some(infer_gmail_send_subject(&message.content, existing_body));
            }
        }

        if to.is_some() && body.is_some() {
            break;
        }
    }

    let to = to?;
    let body = body?;
    let subject = subject.unwrap_or_else(|| infer_gmail_send_subject(last_user_text, &body));
    let safe_body = body.replace('"', "'");
    let safe_subject = subject.replace('"', "'");

    Some(format!(
        "Send \"{}\" mail to {} subject \"{}\"",
        safe_body, to, safe_subject
    ))
}

pub(super) fn extract_attachment_path_from_user_text(user_text: &str) -> Option<String> {
    const ATTACHMENT_PATH_MARKER: &str = "Attachment path (available to local tools if needed):";
    user_text
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(ATTACHMENT_PATH_MARKER)
                .map(|path| path.trim().trim_matches('`').to_string())
        })
        .filter(|path| !path.is_empty())
}

pub(super) fn query_contains_path_like_token(query: &str) -> bool {
    query.split_whitespace().any(|token| {
        token.starts_with('/') || token.starts_with("~/") || token.starts_with("file://")
    })
}

pub(super) fn resolve_intent_fallback_query(
    last_user_text: &str,
    messages: &[ChatMessage],
) -> String {
    // Strip any #tool: directive prefix so it never leaks into tool argument builders.
    // The live-fact classifier rewrites routing_focus_text to "#tool:searxng_search <query>"
    // but the fallback query must be the clean user text, not the directive-prefixed string.
    let clean_user_text = if let Some((_, cleaned)) = extract_forced_tool_directive(last_user_text)
    {
        if !cleaned.is_empty() {
            cleaned
        } else {
            last_user_text.trim().to_string()
        }
    } else {
        last_user_text.trim().to_string()
    };

    let mut resolved = infer_confirmation_send_query_from_history(&clean_user_text, messages)
        .unwrap_or_else(|| clean_user_text.clone());

    let lower = resolved.to_ascii_lowercase();
    let looks_like_image_query = looks_like_pure_image_analysis_request(&lower)
        || lower.contains("image")
        || lower.contains("photo")
        || lower.contains("picture")
        || lower.contains("scan");

    if looks_like_image_query && !query_contains_path_like_token(&resolved) {
        if let Some(path) = messages
            .iter()
            .rev()
            .find(|m| m.role.eq_ignore_ascii_case("user"))
            .and_then(|m| extract_attachment_path_from_user_text(&m.content))
        {
            resolved.push(' ');
            resolved.push_str(&path);
        }
    }

    resolved
}

pub(super) fn looks_like_vision_unavailable_error(error_message: &str) -> bool {
    let lower = error_message.to_ascii_lowercase();
    lower.contains("local llm vision unavailable")
        || lower.contains("image input is not supported")
        || (lower.contains("mmproj") && lower.contains("image"))
        || (lower.contains("mmproj") && lower.contains("vision"))
}
