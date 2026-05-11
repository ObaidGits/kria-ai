use super::*;

// ─── App-lifecycle intent extractors ─────────────────────────────────────────

/// Extract the application name from utterances like "open chrome", "launch vscode", "start spotify".
pub(super) fn extract_app_name_from_query(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let prefixes = ["open ", "launch ", "start ", "run "];
    for prefix in prefixes {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let name = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// Extract a bare https?:// URL from the query text.
pub(super) fn extract_url_from_query(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(|s| {
            s.trim_end_matches(&['.', ',', ')', ']', ';'][..])
                .to_string()
        })
}

/// Extract (search_query, optional_site) from utterances like:
/// - "open Chrome and search for lo-fi music"
/// - "search YouTube for relaxing music"
/// - "play Shape of You on YouTube"
pub(super) fn extract_browser_search_intent(text: &str) -> (String, Option<String>) {
    let lower = text.to_lowercase();

    // Detect site preference.
    let site: Option<String> = if lower.contains("youtube") || lower.contains(" yt ") {
        Some("youtube".into())
    } else {
        None
    };

    // Strip out the site/app name and leading verb phrases, leaving the actual query.
    let after_verb = [
        "search for ",
        "search ",
        "google ",
        "look up ",
        "find ",
        "play ",
    ]
    .iter()
    .find_map(|prefix| {
        lower
            .find(prefix)
            .map(|i| text[i + prefix.len()..].trim().to_string())
    });

    let query = after_verb.unwrap_or_else(|| {
        // Fallback: strip "open <app> and" prefix, take the rest.
        let s = lower
            .strip_prefix("open ")
            .and_then(|s| s.split_once(" and ").map(|x| x.1))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| text.trim().to_string());
        s
    });

    // Remove "on youtube", "on chrome", "in browser" suffixes.
    let suffixes = [
        " on youtube",
        " on chrome",
        " on firefox",
        " in browser",
        " on youtube.com",
        " in youtube",
    ];
    let clean_query = suffixes.iter().fold(query.to_lowercase(), |q, suf| {
        q.trim_end_matches(suf.trim()).trim().to_string()
    });

    let final_query = if clean_query.is_empty() {
        text.trim().to_string()
    } else {
        clean_query
    };
    (final_query, site)
}

/// Extract (app, contact_name, body) from utterances like:
/// - "WhatsApp Anjali 'are you free?'"
/// - "text Anjali hey"
/// - "send a WhatsApp to Anjali saying hello"
pub(super) fn extract_send_message_intent(text: &str) -> (String, String, String) {
    let lower = text.to_lowercase();

    // Detect messaging app.
    let app = if lower.contains("telegram") {
        "telegram"
    } else if lower.contains("signal") {
        "signal"
    } else if lower.contains("gmail") || lower.contains("email") || lower.contains("mail") {
        "gmail"
    } else {
        "whatsapp" // default
    };

    // Find contact name — the first capitalised word after the verb / app name.
    // This is a best-effort heuristic; proper NLP lives in the LLM layer.
    let trigger_words = ["to ", "message ", "text ", "msg "];
    let contact_start = trigger_words
        .iter()
        .find_map(|tw| lower.find(tw).map(|i| i + tw.len()));

    let (contact, body_start_idx) = if let Some(start) = contact_start {
        let words: Vec<&str> = text[start..].split_whitespace().collect();
        let name = words.first().copied().unwrap_or("").to_string();
        let body_start = start + name.len() + 1;
        (name, body_start.min(text.len()))
    } else {
        (String::new(), text.len())
    };

    // Rest of the text after the contact name is the message body.
    let body_raw = text[body_start_idx..].trim();
    // Strip common connective words.
    let body = ["saying ", "say ", "with message ", "message "]
        .iter()
        .find_map(|prefix| body_raw.strip_prefix(prefix))
        .unwrap_or(body_raw)
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string();

    (app.to_string(), contact, body)
}

pub(super) fn infer_file_search_kind(text_lower: &str) -> &'static str {
    if text_lower.contains("folder") || text_lower.contains("directory") {
        "dir"
    } else if text_lower.contains("file") {
        "file"
    } else {
        "any"
    }
}

pub(super) fn infer_file_search_root(text_lower: &str) -> String {
    if [
        "this project",
        "this repo",
        "current project",
        "current directory",
        "here",
    ]
    .iter()
    .any(|needle| text_lower.contains(needle))
    {
        return ".".into();
    }

    std::env::var("HOME").unwrap_or_else(|_| "/home".into())
}

pub(super) fn infer_title(user_text: &str, default_title: &str) -> String {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return default_title.to_string();
    }

    if let Some(caps) = QUOTED_TEXT_RE.captures(trimmed) {
        if let Some(matched) = caps.get(1).or_else(|| caps.get(2)) {
            let title = matched.as_str().trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }

    if let Some(caps) = TITLE_MARKER_RE.captures(trimmed) {
        if let Some(matched) = caps.get(1) {
            let title = matched
                .as_str()
                .trim()
                .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
                .trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }

    if let Some(title) = infer_title_from_creation_context(trimmed) {
        return title;
    }

    default_title.to_string()
}

pub(super) fn clean_title_candidate(candidate: &str) -> String {
    let mut title = candidate
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
        .trim()
        .to_string();

    loop {
        let before = title.clone();
        for prefix in ["a ", "an ", "the "] {
            if title.to_ascii_lowercase().starts_with(prefix) {
                title = title[prefix.len()..].trim_start().to_string();
            }
        }
        if title == before {
            break;
        }
    }

    for suffix in [" please", " now", " for me"] {
        while title.to_ascii_lowercase().ends_with(suffix) {
            title = title[..title.len().saturating_sub(suffix.len())]
                .trim_end()
                .to_string();
        }
    }

    title
}

pub(super) fn infer_title_from_creation_context(user_text: &str) -> Option<String> {
    if !CREATE_TITLE_CONTEXT_RE.is_match(user_text) {
        return None;
    }

    let caps = CREATE_TITLE_FALLBACK_RE.captures(user_text)?;
    let candidate = caps.get(1)?.as_str();
    let title = clean_title_candidate(candidate);
    if title.is_empty() {
        return None;
    }

    let lower = title.to_ascii_lowercase();
    if TITLE_DURATION_ONLY_RE.is_match(&lower)
        || ["today", "tomorrow", "next week", "this week"]
            .iter()
            .any(|kw| lower == *kw)
    {
        return None;
    }

    Some(title)
}

pub(super) fn infer_calendar_time(text_lower: &str) -> Option<(u32, u32)> {
    if let Some(caps) = CALENDAR_TIME_AMPM_RE.captures(text_lower) {
        let hour_raw = caps.get(1)?.as_str().parse::<u32>().ok()?;
        let minute = caps
            .get(2)
            .and_then(|m| m.as_str().parse::<u32>().ok())
            .unwrap_or(0);
        let ampm = caps.get(3)?.as_str().to_ascii_lowercase();

        let mut hour = hour_raw.min(12);
        if ampm == "am" {
            if hour == 12 {
                hour = 0;
            }
        } else if hour != 12 {
            hour += 12;
        }
        return Some((hour.min(23), minute.min(59)));
    }

    if let Some(caps) = CALENDAR_TIME_24H_RE.captures(text_lower) {
        let hour = caps.get(1)?.as_str().parse::<u32>().ok()?.min(23);
        let minute = caps.get(2)?.as_str().parse::<u32>().ok()?.min(59);
        return Some((hour, minute));
    }

    None
}

pub(super) fn looks_like_google_workspace_request(text_lower: &str) -> bool {
    [
        "google workspace",
        "gmail",
        "gmails",
        "inbox",
        "calendar",
        "google meet",
        "gmeet",
        "meet link",
        "google drive",
        "drive",
        "google doc",
        "google docs",
        "document",
        "google sheet",
        "google sheets",
        "spreadsheet",
        "google slides",
        "slides",
        "presentation",
        "google forms",
        "google form",
        "forms",
    ]
    .iter()
    .any(|needle| text_lower.contains(needle))
}

pub(super) fn looks_like_colab_request(text_lower: &str) -> bool {
    [
        "colab",
        "google colab",
        "notebook",
        "jupyter",
        "python notebook",
        "cell",
        "run code",
        "train model",
    ]
    .iter()
    .any(|needle| text_lower.contains(needle))
}

pub(super) fn routing_focus_text_from_user_content(user_text: &str) -> String {
    const IMAGE_PROMPT_MARKER: &str = "\n\nImage attachment is already included for this turn.";

    if let Some((prefix, _)) = user_text.split_once(IMAGE_PROMPT_MARKER) {
        let trimmed = prefix.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    user_text.trim().to_string()
}

pub(super) fn looks_like_pure_image_analysis_request(text_lower: &str) -> bool {
    let has_image_context = ["image", "photo", "picture", "screenshot", "screen", "scan"]
        .iter()
        .any(|needle| text_lower.contains(needle));

    let has_analysis_intent = [
        "analy",
        "describe",
        "what is",
        "what's in",
        "identify",
        "detect",
        "read",
        "extract",
        "ocr",
        "summar",
    ]
    .iter()
    .any(|needle| text_lower.contains(needle));

    let has_non_image_action = [
        "gmail",
        "email",
        "calendar",
        "drive",
        "doc",
        "spreadsheet",
        "sheet",
        "slides",
        "form",
        "install",
        "uninstall",
        "delete",
        "remove",
        "rename",
        "move",
        "copy",
        "web search",
        "news",
        "git",
    ]
    .iter()
    .any(|needle| text_lower.contains(needle));

    has_image_context && has_analysis_intent && !has_non_image_action
}

pub(super) fn infer_image_analysis_intent_hint(user_query: &str) -> &'static str {
    let lower = user_query.to_ascii_lowercase();
    let has_text = ["ocr", "read", "extract", "text", "word", "sentence"]
        .iter()
        .any(|needle| lower.contains(needle));
    let has_scene = [
        "describe", "scene", "object", "identify", "detect", "analy", "what is",
    ]
    .iter()
    .any(|needle| lower.contains(needle));

    match (has_text, has_scene) {
        (true, true) => "mixed",
        (true, false) => "text_reading",
        (false, true) => "scene_understanding",
        (false, false) => "mixed",
    }
}

pub(super) fn is_tool_allowed_for_image_focus(def: &ToolDef) -> bool {
    if def.category.eq_ignore_ascii_case("vision") {
        return true;
    }

    let name = def.name.to_ascii_lowercase();
    name.contains("image")
        || name.contains("ocr")
        || name.contains("vision")
        || name == "screenshot_analyze"
}

pub(super) fn looks_like_drive_list_request(text_lower: &str) -> bool {
    let has_drive_context = ["google drive", "drive"]
        .iter()
        .any(|needle| text_lower.contains(needle));
    let has_list_intent = [
        "list",
        "show",
        "browse",
        "contents",
        "what is in",
        "what's in",
    ]
    .iter()
    .any(|needle| text_lower.contains(needle));
    let has_search_intent = ["search", "find", "look for", "locate"]
        .iter()
        .any(|needle| text_lower.contains(needle));

    has_drive_context && has_list_intent && !has_search_intent
}

pub(super) fn infer_calendar_duration_minutes(text_lower: &str) -> i64 {
    if text_lower.contains("half hour") {
        return 30;
    }

    if let Some(caps) = CALENDAR_DURATION_RE.captures(text_lower) {
        if let Some(value) = caps.get(1).and_then(|m| m.as_str().parse::<i64>().ok()) {
            let unit = caps
                .get(2)
                .map(|m| m.as_str().to_ascii_lowercase())
                .unwrap_or_default();
            if unit.starts_with('h') {
                return (value * 60).clamp(15, 8 * 60);
            }
            return value.clamp(15, 8 * 60);
        }
    }

    60
}

pub(super) fn infer_calendar_window(user_text: &str) -> Option<(String, String)> {
    let lower = user_text.to_lowercase();
    let day_offset = if lower.contains("day after tomorrow") {
        2
    } else if lower.contains("tomorrow") {
        1
    } else if lower.contains("today") {
        0
    } else if lower.contains("next week") {
        7
    } else {
        return None;
    };

    let base_date = Local::now().date_naive() + Duration::days(day_offset);
    let (hour, minute) = infer_calendar_time(&lower).unwrap_or((9, 0));

    let start = Local
        .with_ymd_and_hms(
            base_date.year(),
            base_date.month(),
            base_date.day(),
            hour,
            minute,
            0,
        )
        .single()?;
    let end = start + Duration::minutes(infer_calendar_duration_minutes(&lower));

    let start_utc = start.with_timezone(&Utc);
    let end_utc = end.with_timezone(&Utc);
    Some((
        start_utc.to_rfc3339_opts(SecondsFormat::Secs, true),
        end_utc.to_rfc3339_opts(SecondsFormat::Secs, true),
    ))
}

pub(super) fn infer_calendar_attendees(user_text: &str) -> Vec<String> {
    let mut attendees = Vec::new();
    for caps in CALENDAR_ATTENDEE_EMAIL_RE.captures_iter(user_text) {
        if let Some(matched) = caps.get(1) {
            let email = matched.as_str().trim().to_ascii_lowercase();
            if !email.is_empty() && !attendees.iter().any(|e: &String| e == &email) {
                attendees.push(email);
            }
        }
    }
    attendees
}

pub(super) fn infer_calendar_summary(user_text: &str) -> String {
    let explicit = infer_title(user_text, "");
    if !explicit.is_empty() {
        return explicit;
    }

    let lower = user_text.to_lowercase();
    if lower.contains("google meet") || lower.contains("gmeet") || lower.contains("meet") {
        return "Google Meet".into();
    }
    if lower.contains("interview") {
        return "Interview".into();
    }
    if lower.contains("appointment") {
        return "Appointment".into();
    }
    if lower.contains("call") {
        return "Call".into();
    }
    if lower.contains("meeting") {
        return "Meeting".into();
    }

    "New Event".into()
}

pub(super) fn infer_calendar_create_arguments(user_text: &str) -> Option<serde_json::Value> {
    let (start, end) = infer_calendar_window(user_text)?;
    let lower = user_text.to_lowercase();
    let attendees = infer_calendar_attendees(user_text);

    let mut args = serde_json::json!({
        "summary": infer_calendar_summary(user_text),
        "start": start,
        "end": end,
        "description": if lower.contains("google meet") || lower.contains("gmeet") || lower.contains("meet link") {
            "Requested via KRIA (Google Meet)"
        } else {
            ""
        },
        "location": "",
    });

    if !attendees.is_empty() {
        args["attendees"] = serde_json::Value::Array(
            attendees
                .into_iter()
                .map(|email| serde_json::json!({ "email": email }))
                .collect(),
        );
    }

    Some(args)
}

pub(super) fn infer_file_search_target(user_text: &str) -> Option<String> {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(caps) = QUOTED_TEXT_RE.captures(trimmed) {
        if let Some(matched) = caps.get(1).or_else(|| caps.get(2)) {
            let target = matched.as_str().trim();
            if !target.is_empty() {
                return Some(target.to_string());
            }
        }
    }

    if let Some(caps) = FILE_SEARCH_MARKER_RE.captures(trimmed) {
        if let Some(matched) = caps.get(1) {
            let target = matched
                .as_str()
                .trim()
                .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim();
            if !target.is_empty() {
                return Some(target.to_string());
            }
        }
    }

    None
}

pub(super) fn extract_forced_tool_directive(user_text: &str) -> Option<(String, String)> {
    let caps = FORCED_TOOL_DIRECTIVE_RE.captures(user_text.trim())?;
    let tool = caps.get(1)?.as_str().trim().to_string();
    if tool.is_empty() {
        return None;
    }
    let query = caps
        .get(2)
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_default();
    Some((tool, query))
}
