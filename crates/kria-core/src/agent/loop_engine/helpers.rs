use once_cell::sync::Lazy;
use regex::Regex;
use std::hash::{Hash, Hasher};

/// Compute a stable u64 hash for a `(tool_name, arguments)` pair.
/// Used to detect duplicate failed tool calls within a single turn.
pub(super) fn call_dedup_hash(tool_name: &str, arguments: &serde_json::Value) -> u64 {
    // Canonicalize argument JSON (sort keys) so {"a":1,"b":2} == {"b":2,"a":1}.
    let canonical_args = canonical_json(arguments);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tool_name.hash(&mut h);
    canonical_args.hash(&mut h);
    h.finish()
}

/// Serialize a `serde_json::Value` with object keys sorted for stable comparison.
pub(super) fn canonical_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let mut pairs: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            pairs.sort_by_key(|(k, _)| k.as_str());
            let inner = pairs
                .iter()
                .map(|(k, v)| format!("\"{}\":{}", k, canonical_json(v)))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
        serde_json::Value::Array(arr) => {
            let inner = arr.iter().map(canonical_json).collect::<Vec<_>>().join(",");
            format!("[{inner}]")
        }
        other => other.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PackageIntent {
    Install,
    Uninstall,
}

pub(super) static REQUESTED_LIMIT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:top|last|latest|recent|first|show|get|fetch|read|check)\s+(\d{1,3})\b|\b(\d{1,3})\s+(?:unread|emails?|messages?|results?|files?|folders?|directories?)\b",
    )
    .expect("valid requested limit regex")
});

pub(super) static QUOTED_TEXT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#""([^"]+)"|'([^']+)'"#).expect("valid quoted text regex"));

pub(super) static FILE_SEARCH_MARKER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:file|folder|directory)(?:\s+name)?\s+(?:named|called)?\s*([^\n\r,.;!?]+)")
        .expect("valid file search marker regex")
});

pub(super) static TITLE_MARKER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:called|named|titled|title)\s+([^\n\r,.;!?]+)")
        .expect("valid title marker regex")
});

pub(super) static CREATE_TITLE_CONTEXT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:create|new|start|make|build|draft|write)\b.*\b(?:google\s+(?:doc|docs|sheet|sheets|slides|form|forms)|document|spreadsheet|presentation|deck|form)\b",
    )
    .expect("valid title context regex")
});

pub(super) static CREATE_TITLE_FALLBACK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:for|about)\s+([^\n\r,.;!?]+)").expect("valid title fallback regex")
});

pub(super) static TITLE_DURATION_ONLY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\d{1,3}\s*(?:minute|minutes|min|hour|hours|hr|hrs)\b")
        .expect("valid title duration regex")
});

pub(super) static CALENDAR_TIME_AMPM_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(\d{1,2})(?::(\d{2}))?\s*(am|pm)\b").expect("valid calendar ampm time regex")
});

pub(super) static CALENDAR_TIME_24H_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b([01]?\d|2[0-3]):([0-5]\d)\b").expect("valid calendar 24h time regex")
});

pub(super) static CALENDAR_DURATION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bfor\s+(\d{1,3})\s*(minute|minutes|min|hour|hours|hr|hrs)\b")
        .expect("valid calendar duration regex")
});

pub(super) static CALENDAR_ATTENDEE_EMAIL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b([a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,})\b")
        .expect("valid calendar attendee email regex")
});

pub(super) static GMAIL_SEND_BODY_BEFORE_MAIL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:send|write|compose|draft)\b\s+(?:an?\s+|the\s+)?(.+?)\s+\b(?:mail|email|gmail)\b",
    )
    .expect("valid gmail send body-before-mail regex")
});

pub(super) static GMAIL_SEND_BODY_AFTER_SAYING_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:saying|say|with\s+message|message\s+is)\b\s+(.+?)(?:\s+\bto\b|$)")
        .expect("valid gmail send body-after-saying regex")
});

pub(super) static GMAIL_SEND_SUBJECT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bsubject\b\s*(?::|is)?\s+([^\n\r,;!?]+)")
        .expect("valid gmail send subject regex")
});

pub(super) static GMAIL_MESSAGE_ID_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:message[_\s-]?id|gmail[_\s-]?id|email[_\s-]?id)\b\s*[:=]?\s*([A-Za-z0-9_-]{10,})")
        .expect("valid gmail message id regex")
});

pub(super) static CALENDAR_EVENT_ID_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:event[_\s-]?id|calendar[_\s-]?event[_\s-]?id)\b\s*[:=]?\s*([A-Za-z0-9_@-]{8,})",
    )
    .expect("valid calendar event id regex")
});

pub(super) static GENERIC_RESOURCE_ID_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:file[_\s-]?id|document[_\s-]?id|spreadsheet[_\s-]?id|presentation[_\s-]?id|id)\b\s*[:=]?\s*([A-Za-z0-9_-]{10,})")
        .expect("valid generic resource id regex")
});

pub(super) static SHEETS_RANGE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b([A-Za-z0-9_]+![A-Z]+\d+(?::[A-Z]+\d+)?|[A-Z]+\d+(?::[A-Z]+\d+)?)\b")
        .expect("valid sheets range regex")
});

pub(super) static APPEND_TEXT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:append|add|insert|write)\b\s+(.+?)(?:\s+\b(?:to|into|in)\b|$)")
        .expect("valid append text regex")
});

pub(super) static SEND_CONFIRMATION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^\s*(?:yes\s*,?\s*)?(?:send(?:\s+it)?|go\s+ahead|confirm|proceed)(?:\s+(?:now|immediately|right\s+now))?\s*[.!]?\s*$",
    )
    .expect("valid send confirmation regex")
});

pub(super) static FORCED_TOOL_DIRECTIVE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?is)^\s*#tool:\s*([a-zA-Z0-9_-]+)\s*(.*)$")
        .expect("valid forced tool directive regex")
});

pub(super) static FENCED_CODE_BLOCK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?is)```(?:[a-z0-9_+\-]+)?\s*(.*?)\s*```").expect("valid fenced code block regex")
});

pub(super) static SENSITIVE_JSON_FIELD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)"([^"]*(?:api[_-]?key|access[_-]?token|refresh[_-]?token|authorization|secret)[^"]*)"\s*:\s*"([^"\n]{12,})""#,
    )
    .expect("valid sensitive json field regex")
});

pub(super) static MULTI_NEWLINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\n{3,}").expect("valid multi newline regex"));

pub(super) static REMOTE_VM_INDEX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bvm\s*#?\s*(\d{1,3})\b").expect("valid remote vm index regex"));

pub(super) static REMOTE_VM_TARGET_HINT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:on|in)\s+(?:my\s+)?(?:local\s+)?vm\s+([a-z0-9][a-z0-9_.:-]{0,63})\b")
        .expect("valid remote vm target hint regex")
});

pub(super) static REMOTE_CONNECTED_TARGET_HINT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:on|in)\s+(?:my\s+)?connected\s+(?:computer|machine|laptop|pc|host)\s+([a-z0-9][a-z0-9_.:-]{0,63})\b",
    )
    .expect("valid connected target hint regex")
});

pub(super) static REMOTE_PACKAGE_LIST_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:what|which|show|list|check|tell)\b.{0,40}\b(?:apps?|applications?|packages?|programs?)\b.{0,24}\b(?:installed|install)\b|\b(?:installed|install)\b.{0,24}\b(?:apps?|applications?|packages?|programs?)\b|\b(?:apps?|applications?|packages?|programs?)\b.{0,24}\b(?:installed|install)\b",
    )
    .expect("valid remote package list regex")
});

pub(super) fn is_remote_command_context(user_text: &str) -> bool {
    let text = user_text.to_ascii_lowercase();
    text.contains(" via ssh")
        || text.starts_with("ssh ")
        || text.contains(" on my vm")
        || text.contains(" on vm")
        || text.contains(" in my vm")
        || text.contains(" in vm")
        || text.contains(" local vm")
        || text.contains(" remote vm")
        || text.contains(" remote host")
        || text.contains(" remote computer")
        || text.contains(" remote laptop")
        || text.contains(" connected computer")
        || text.contains(" connected machine")
        || text.contains(" connected laptop")
        || text.contains(" connected pc")
        || REMOTE_VM_INDEX_RE.is_match(&text)
}

pub(super) fn normalize_inferred_target_hint(raw: &str) -> Option<String> {
    let cleaned = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | ':' | '.' | '(' | ')'));
    if cleaned.is_empty() {
        return None;
    }

    if cleaned.chars().all(|c| c.is_ascii_digit()) {
        return Some(format!("vm {}", cleaned));
    }

    Some(cleaned.to_string())
}

pub(super) fn infer_remote_target_hint(user_text: &str) -> Option<String> {
    if let Some(caps) = REMOTE_VM_INDEX_RE.captures(user_text) {
        if let Some(index) = caps.get(1) {
            return Some(format!("vm {}", index.as_str()));
        }
    }

    if let Some(caps) = REMOTE_VM_TARGET_HINT_RE.captures(user_text) {
        if let Some(value) = caps.get(1) {
            return normalize_inferred_target_hint(value.as_str());
        }
    }

    if let Some(caps) = REMOTE_CONNECTED_TARGET_HINT_RE.captures(user_text) {
        if let Some(value) = caps.get(1) {
            return normalize_inferred_target_hint(value.as_str());
        }
    }

    None
}

pub(super) fn is_remote_package_listing_request(user_text: &str) -> bool {
    is_remote_command_context(user_text) && REMOTE_PACKAGE_LIST_RE.is_match(user_text)
}

pub(super) fn build_remote_package_listing_command() -> String {
    "if command -v apt >/dev/null 2>&1; then apt list --installed 2>/dev/null; \
elif command -v dnf >/dev/null 2>&1; then dnf list installed; \
elif command -v pacman >/dev/null 2>&1; then pacman -Q; \
elif command -v zypper >/dev/null 2>&1; then zypper search --installed-only; \
elif command -v brew >/dev/null 2>&1; then brew list --versions; \
elif command -v snap >/dev/null 2>&1; then snap list; \
elif command -v flatpak >/dev/null 2>&1; then flatpak list --app; \
else echo 'No supported package manager found on remote target' >&2; exit 127; fi"
        .to_string()
}

pub(super) fn detect_package_intent_raw(user_text: &str) -> Option<PackageIntent> {
    let text = user_text.to_lowercase();
    if ["uninstall", "remove", "delete package"]
        .iter()
        .any(|m| text.contains(m))
    {
        return Some(PackageIntent::Uninstall);
    }
    if ["install", "setup", "set up"]
        .iter()
        .any(|m| text.contains(m))
    {
        return Some(PackageIntent::Install);
    }
    None
}

/// True when an install/uninstall request is about a KRIA *capability*
/// (skill/tool/plugin from the Capability Provider Platform marketplace) rather
/// than an operating-system software package. Such requests must NOT enter the
/// OS package-manager flow (`search_package`/`install_package`) — they belong to
/// the marketplace tools (`search_marketplace`/`install_capability`). This is an
/// intent-class disambiguation over generic capability nouns, not a per-prompt
/// special case, so it generalizes to unseen requests.
pub(super) fn refers_to_marketplace_capability(user_text: &str) -> bool {
    let text = user_text.to_lowercase();
    [
        "tool",
        "skill",
        "capability",
        "plugin",
        "add-on",
        "addon",
        "extension",
        "integration",
        "marketplace",
        "openclaw",
        "clawhub",
    ]
    .iter()
    .any(|noun| text.contains(noun))
}

pub(super) fn detect_package_intent(user_text: &str) -> Option<PackageIntent> {
    if is_remote_command_context(user_text) {
        return None;
    }
    // Capability/skill/tool installs are a marketplace concern, not an OS
    // package-manager concern — don't hijack them into the package flow.
    if refers_to_marketplace_capability(user_text) {
        return None;
    }
    detect_package_intent_raw(user_text)
}

/// Extract the descriptive capability query from a marketplace request by
/// stripping the lead-in verb ("install", "search the marketplace for", ...) and
/// trailing filler ("from the marketplace", "please"). The remainder (e.g. "ip
/// info tool", "pdf extractor") is what the provider's marketplace matcher ranks
/// against — general, no per-skill logic.
pub(super) fn extract_capability_query(user_text: &str) -> Option<String> {
    let lower = user_text.to_lowercase();
    // Most-specific markers first so "search the marketplace for X" wins over
    // the bare "search for" / "for" fragments.
    let markers: &[&str] = &[
        "search the marketplace for ",
        "search marketplace for ",
        "look in the marketplace for ",
        "install a new ",
        "install the ",
        "install a ",
        "install an ",
        "install ",
        "add a ",
        "add an ",
        "add the ",
        "add ",
        "get me a ",
        "get me an ",
        "get me the ",
        "get me ",
        "download the ",
        "download a ",
        "download ",
        "set up a ",
        "set up ",
        "setup ",
        "enable the ",
        "enable ",
        "find me a ",
        "find me an ",
        "find a ",
        "find an ",
        "find ",
        "search for ",
        "search ",
        "look for a ",
        "look for ",
        "browse for ",
        "browse ",
    ];
    let mut frag = extract_after_first_marker(&lower, markers)
        .unwrap_or(lower.as_str())
        .trim()
        .to_string();
    frag = frag
        .split(|c: char| ".,!?;:\n".contains(c))
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    for prefix in ["the ", "a ", "an ", "new ", "some "] {
        if frag.starts_with(prefix) {
            frag = frag[prefix.len()..].trim_start().to_string();
        }
    }
    for suffix in [
        " from the marketplace",
        " in the marketplace",
        " from marketplace",
        " on the marketplace",
        " please",
        " now",
        " for me",
        " thanks",
    ] {
        if let Some(idx) = frag.find(suffix) {
            frag.truncate(idx);
        }
    }
    let frag = frag.trim().to_string();
    if frag.is_empty() {
        None
    } else {
        Some(frag)
    }
}

pub(super) fn normalize_package_query(raw: &str) -> String {
    let cleaned = raw.trim().to_lowercase();
    match cleaned.as_str() {
        "chrome"
        | "google chrome"
        | "google-chrome"
        | "google-chrome-stable"
        | "chrome browser"
        | "google chrome browser" => "chromium".into(),
        _ => cleaned,
    }
}

pub(super) fn extract_after_first_marker<'a>(text: &'a str, markers: &[&str]) -> Option<&'a str> {
    for marker in markers {
        if let Some(idx) = text.find(marker) {
            let start = idx + marker.len();
            return text.get(start..);
        }
    }
    None
}

pub(super) fn extract_package_query(user_text: &str, intent: PackageIntent) -> Option<String> {
    let lower = user_text.to_lowercase();
    let markers: &[&str] = match intent {
        PackageIntent::Install => &["install ", "setup ", "set up "],
        PackageIntent::Uninstall => &["uninstall ", "remove ", "delete "],
    };

    let mut fragment = extract_after_first_marker(&lower, markers)?
        .split(|c: char| ".,!?;:\n".contains(c))
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    loop {
        let before = fragment.clone();
        for prefix in [
            "the ",
            "a ",
            "an ",
            "package ",
            "app ",
            "application ",
            "software ",
        ] {
            if fragment.starts_with(prefix) {
                fragment = fragment[prefix.len()..].trim_start().to_string();
            }
        }
        if fragment == before {
            break;
        }
    }

    for suffix in [" please", " now", " for me", " thanks", " thank you"] {
        while fragment.ends_with(suffix) {
            fragment = fragment[..fragment.len() - suffix.len()]
                .trim_end()
                .to_string();
        }
    }

    if fragment.is_empty() {
        return None;
    }

    // Keep the query compact but preserve 2-word app names like "google chrome".
    let mut words = fragment.split_whitespace();
    let first = words.next()?;
    let second = words.next();
    let compact = if matches!(second, Some("chrome")) && first == "google" {
        format!("{first} chrome")
    } else {
        first.to_string()
    };
    Some(normalize_package_query(&compact))
}

pub(super) fn sanitize_package_name_for_shell(raw: &str) -> Option<String> {
    let cleaned = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(*c, '-' | '_' | '.' | '+'))
        .collect::<String>();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

pub(super) fn build_remote_package_manager_command(
    intent: PackageIntent,
    package_query: &str,
) -> Option<String> {
    let package = sanitize_package_name_for_shell(&normalize_package_query(package_query))?;
    let command = match intent {
        PackageIntent::Install => format!(
            "if command -v apt-get >/dev/null 2>&1; then sudo -n apt-get update && sudo -n apt-get install -y {package}; \
elif command -v dnf >/dev/null 2>&1; then sudo -n dnf install -y {package}; \
elif command -v pacman >/dev/null 2>&1; then sudo -n pacman -S --noconfirm {package}; \
elif command -v zypper >/dev/null 2>&1; then sudo -n zypper --non-interactive install {package}; \
elif command -v brew >/dev/null 2>&1; then brew install {package}; \
elif command -v snap >/dev/null 2>&1; then sudo -n snap install {package}; \
elif command -v flatpak >/dev/null 2>&1; then flatpak install -y --user {package} || sudo -n flatpak install -y {package}; \
else echo 'No supported package manager found on remote target' >&2; exit 127; fi"
        ),
        PackageIntent::Uninstall => format!(
            "if command -v apt-get >/dev/null 2>&1; then sudo -n apt-get remove -y {package}; \
elif command -v dnf >/dev/null 2>&1; then sudo -n dnf remove -y {package}; \
elif command -v pacman >/dev/null 2>&1; then sudo -n pacman -R --noconfirm {package}; \
elif command -v zypper >/dev/null 2>&1; then sudo -n zypper --non-interactive remove {package}; \
elif command -v brew >/dev/null 2>&1; then brew uninstall {package}; \
elif command -v snap >/dev/null 2>&1; then sudo -n snap remove {package}; \
elif command -v flatpak >/dev/null 2>&1; then flatpak uninstall -y --user {package} || sudo -n flatpak uninstall -y {package}; \
else echo 'No supported package manager found on remote target' >&2; exit 127; fi"
        ),
    };
    Some(command)
}

pub(super) fn extract_ssh_target_and_shell(user_text: &str) -> Option<(Option<String>, String)> {
    let lower = user_text.to_ascii_lowercase();
    let ssh_start = lower.find("ssh ")?;
    let ssh_segment = user_text.get(ssh_start..)?.trim();

    let target_hint = ssh_segment.split_whitespace().skip(1).find_map(|token| {
        let trimmed = token.trim_matches(|c: char| c == '"' || c == '\'' || c == ',' || c == ';');
        if trimmed.is_empty() || trimmed.starts_with('-') {
            return None;
        }
        if let Some((_, host)) = trimmed.split_once('@') {
            return Some(host.trim_end_matches(':').to_string());
        }
        if trimmed.contains('.') || trimmed.contains(':') {
            return Some(trimmed.to_string());
        }
        None
    });

    let mut last_quoted: Option<String> = None;
    for captures in QUOTED_TEXT_RE.captures_iter(ssh_segment) {
        if let Some(value) = captures.get(1).or_else(|| captures.get(2)) {
            let text = value.as_str().trim();
            if !text.is_empty() {
                last_quoted = Some(text.to_string());
            }
        }
    }

    if let Some(command) = last_quoted {
        return Some((target_hint, command));
    }

    let ssh_passthrough = ssh_segment.to_string();
    Some((target_hint, ssh_passthrough))
}

pub(super) fn extract_ssh_passthrough_command(user_text: &str) -> Option<String> {
    let lower = user_text.to_ascii_lowercase();
    let ssh_start = lower.find("ssh ")?;
    user_text
        .get(ssh_start..)
        .map(|value| value.trim().to_string())
}

pub(super) fn extract_remote_command_request(user_text: &str) -> Option<(String, Option<String>)> {
    if let Some((target_hint, shell)) = extract_ssh_target_and_shell(user_text) {
        let shell = shell.trim();
        if !shell.is_empty() {
            let command = if shell.to_ascii_lowercase().starts_with("ssh ") {
                // If we only have a passthrough SSH command, keep it unchanged.
                shell.to_string()
            } else {
                shell.to_string()
            };
            return Some((command, target_hint));
        }
    }

    let lower = user_text.to_ascii_lowercase();
    let inferred_target_hint = infer_remote_target_hint(user_text);
    for marker in [
        "run on my vm:",
        "run on vm:",
        "execute on my vm:",
        "execute on vm:",
        "remote command:",
    ] {
        if let Some(idx) = lower.find(marker) {
            let rest = user_text[idx + marker.len()..].trim();
            if !rest.is_empty() {
                return Some((rest.to_string(), inferred_target_hint.clone()));
            }
        }
    }

    if !is_remote_command_context(user_text) {
        return None;
    }

    if is_remote_package_listing_request(user_text) {
        return Some((build_remote_package_listing_command(), inferred_target_hint));
    }

    if let Some(intent) = detect_package_intent_raw(user_text) {
        if let Some(package_query) = extract_package_query(user_text, intent) {
            if let Some(command) = build_remote_package_manager_command(intent, &package_query) {
                return Some((command, inferred_target_hint));
            }
        }
    }

    for captures in QUOTED_TEXT_RE.captures_iter(user_text) {
        if let Some(value) = captures.get(1).or_else(|| captures.get(2)) {
            let text = value.as_str().trim();
            if !text.is_empty() {
                return Some((text.to_string(), inferred_target_hint));
            }
        }
    }

    None
}

pub(super) fn normalize_package_source_for_action(source: &str) -> Option<String> {
    match source.trim().to_lowercase().as_str() {
        "apt" | "dnf" | "pacman" | "zypper" | "brew" | "winget" | "choco" | "snap" | "flatpak" => {
            Some(source.trim().to_lowercase())
        }
        "brew-formula" | "brew-cask" => Some("brew".into()),
        _ => None,
    }
}

pub(super) fn is_sidecar_backed_tool_name(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "search_news"
            | "fetch_article"
            | "list_news_sources"
            | "news_status"
            | "image_analyze"
            | "document_extract"
            | "code_analyze_ast"
            | "web_extract_text"
            | "compute_embeddings"
            | "audio_preprocess"
            | "ocr_image"
            | "analyze_image"
            | "screenshot_analyze"
    )
}

pub(super) fn user_requested_explicit_queue(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    [
        "queue this",
        "queue it",
        "put this in queue",
        "put it in queue",
        "add this to queue",
        "add it to queue",
        "after current",
        "after this",
        "when you're done",
        "when you are done",
        "do not interrupt",
        "don't interrupt",
        "wait your turn",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

pub(super) fn infer_news_country_code(text_lower: &str) -> Option<&'static str> {
    if text_lower.contains("india") || text_lower.contains("indian") {
        return Some("IN");
    }
    if text_lower.contains("pakistan") {
        return Some("PK");
    }
    if text_lower.contains("bangladesh") {
        return Some("BD");
    }
    if text_lower.contains("sri lanka") {
        return Some("LK");
    }
    if text_lower.contains("united states")
        || text_lower.contains(" usa")
        || text_lower.contains(" us ")
    {
        return Some("US");
    }
    if text_lower.contains("united kingdom")
        || text_lower.contains(" uk ")
        || text_lower.contains("britain")
    {
        return Some("GB");
    }
    None
}

pub(super) fn infer_requested_limit(user_text: &str, default: u64, max: u64) -> u64 {
    REQUESTED_LIMIT_RE
        .captures(user_text)
        .and_then(|caps| caps.iter().skip(1).flatten().next())
        .and_then(|m| m.as_str().parse::<u64>().ok())
        .filter(|count| *count > 0)
        .map(|count| count.min(max))
        .unwrap_or(default)
}

pub(super) fn infer_gmail_list_query(user_text: &str) -> String {
    let text_lower = user_text.to_lowercase();
    let mut filters: Vec<&str> = Vec::new();

    if text_lower.contains("sent") {
        filters.push("in:sent");
    } else if text_lower.contains("draft") {
        filters.push("in:drafts");
    } else if text_lower.contains("spam") {
        filters.push("in:spam");
    } else if text_lower.contains("trash") {
        filters.push("in:trash");
    } else {
        filters.push("in:inbox");
    }

    if text_lower.contains("unread") {
        filters.push("is:unread");
    }
    if text_lower.contains("starred") {
        filters.push("is:starred");
    }
    if text_lower.contains("important") {
        filters.push("is:important");
    }

    filters.join(" ")
}

pub(super) fn looks_like_raw_gmail_payload_json(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    if !lower.contains("\"messages\"") {
        return false;
    }

    let has_payload_shape_markers = lower.contains("\"query\"")
        || lower.contains("\"requested_count\"")
        || lower.contains("\"returned_count\"")
        || lower.contains("\"llm_visible_message_count\"")
        || lower.contains("\"count\"")
        || lower.contains("\"fully_satisfied\"")
        || lower.contains("\"has_more_results\"");

    let has_gmail_row_markers = lower.contains("\"from\"")
        || lower.contains("\"subject\"")
        || lower.contains("\"preview\"")
        || lower.contains("\"labels\"")
        || lower.contains("\"date\"")
        || lower.contains("\"id\"");

    has_payload_shape_markers && has_gmail_row_markers
}

pub(super) fn contains_forbidden_payload_markers(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    [
        "\"toolbench_rapidapi_key\"",
        "\"toolbench_rapidapi_url\"",
        "\"x-rapidapi-key\"",
        "\"rapidapi_key\"",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || SENSITIVE_JSON_FIELD_RE.is_match(block)
}

pub(super) fn should_filter_code_block(block: &str) -> bool {
    let trimmed = block.trim();
    let json_like = trimmed.starts_with('{') || trimmed.starts_with('[');
    if !json_like {
        return false;
    }

    contains_forbidden_payload_markers(trimmed) || looks_like_raw_gmail_payload_json(trimmed)
}

pub(super) fn sanitize_assistant_text_response(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }

    let filtered_blocks = FENCED_CODE_BLOCK_RE
        .replace_all(text, |caps: &regex::Captures| {
            let block = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if should_filter_code_block(block) {
                "[Filtered unsafe raw payload omitted.]".to_string()
            } else {
                caps.get(0)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default()
            }
        })
        .to_string();

    let redacted_inline = SENSITIVE_JSON_FIELD_RE
        .replace_all(&filtered_blocks, |caps: &regex::Captures| {
            let key = caps.get(1).map(|m| m.as_str()).unwrap_or("secret");
            format!(r#""{key}": "[REDACTED]""#)
        })
        .to_string();

    MULTI_NEWLINE_RE
        .replace_all(redacted_inline.trim(), "\n\n")
        .to_string()
}

/// Infer a `Verifiability` leaf for post-execution verification.
///
/// Returns `Some(leaf)` for tools whose effects can be verified deterministically.
/// Returns `None` for tools that are trivial (read-only, informational) or
/// whose effects cannot be verified without additional context.
///
/// The verifier NEVER retries or replans — it only validates and logs.
pub(super) fn infer_verifiability_for_tool(
    tool_name: &str,
    params: &serde_json::Value,
    result: &crate::infra::isolation::ToolResult,
) -> Option<crate::agent::execution_verifier::Verifiability> {
    use crate::agent::execution_verifier::{FsEffect, Verifiability};
    use std::path::PathBuf;

    // Only verify successful results — failed results are already handled
    if !result.success {
        return None;
    }

    match tool_name {
        // File write operations: verify the file exists after creation
        "write_file" | "create_file" | "overwrite_file" => {
            let path = params.get("path").and_then(|v| v.as_str())?;
            Some(Verifiability::FileSystemEffect {
                path: PathBuf::from(path),
                kind: FsEffect::Exists,
            })
        }

        // File write with content: verify file exists and has content
        "append_to_file" => {
            let path = params.get("path").and_then(|v| v.as_str())?;
            Some(Verifiability::FileSystemEffect {
                path: PathBuf::from(path),
                kind: FsEffect::SizeGreaterThan(0),
            })
        }

        // Process launch: verify the process is running
        "open_application" | "launch_application" => {
            let app = params
                .get("name")
                .or_else(|| params.get("app"))
                .or_else(|| params.get("application"))
                .and_then(|v| v.as_str())?;
            // Use a short binary name (first word, lowercase)
            let binary = app.split_whitespace().next()?.to_ascii_lowercase();
            Some(Verifiability::ProcessLaunched {
                binary,
                max_wait_ms: 500,
            })
        }

        // File removal: verify the file no longer exists
        "delete_file" | "remove_file" | "rm_file" => {
            let path = params.get("path").and_then(|v| v.as_str())?;
            Some(Verifiability::FileSystemEffect {
                path: PathBuf::from(path),
                kind: FsEffect::NotExists,
            })
        }

        // File move/rename: verify destination exists
        "move_file" | "rename_file" => {
            let dest = params
                .get("destination")
                .or_else(|| params.get("dest"))
                .or_else(|| params.get("to"))
                .and_then(|v| v.as_str())?;
            Some(Verifiability::FileSystemEffect {
                path: PathBuf::from(dest),
                kind: FsEffect::Exists,
            })
        }

        // File copy: verify destination exists
        "copy_file" | "cp_file" => {
            let dest = params
                .get("destination")
                .or_else(|| params.get("dest"))
                .and_then(|v| v.as_str())?;
            Some(Verifiability::FileSystemEffect {
                path: PathBuf::from(dest),
                kind: FsEffect::Exists,
            })
        }

        // Process close: verify the process is no longer running
        "close_application" | "close_window" | "kill_process" => {
            let app = params
                .get("name")
                .or_else(|| params.get("app"))
                .or_else(|| params.get("application"))
                .or_else(|| params.get("process"))
                .and_then(|v| v.as_str())?;
            let binary = app.split_whitespace().next()?.to_ascii_lowercase();
            Some(Verifiability::ProcessNotRunning {
                binary,
                max_wait_ms: 2000,
            })
        }

        // Shell execution: if the result contains a file path, verify it exists
        "execute_bash" | "execute_python" | "execute_fleet_command" => {
            // Look for a file path in the result data
            let output = result
                .data
                .as_str()
                .or_else(|| result.data.get("stdout").and_then(|v| v.as_str()))?;

            // Simple heuristic: if output contains an absolute path that looks like
            // a created file, verify it exists
            let path_line = output.lines().find(|line| {
                let trimmed = line.trim();
                trimmed.starts_with('/') && !trimmed.contains(' ') && trimmed.len() > 3
            })?;

            let path = PathBuf::from(path_line.trim());
            // Only verify if the path looks like a file (has extension or is in /tmp)
            if path.extension().is_some() || path.starts_with("/tmp") {
                Some(Verifiability::FileSystemEffect {
                    path,
                    kind: FsEffect::Exists,
                })
            } else {
                None
            }
        }

        // All other tools: no specific verification
        _ => None,
    }
}

#[cfg(test)]
mod marketplace_intent_tests {
    use super::*;

    #[test]
    fn os_package_install_still_routes_to_package_flow() {
        // Real OS software → OS package manager intent.
        assert_eq!(
            detect_package_intent("install htop"),
            Some(PackageIntent::Install)
        );
        assert_eq!(
            detect_package_intent("please install docker for me"),
            Some(PackageIntent::Install)
        );
    }

    #[test]
    fn capability_installs_are_excluded_from_os_package_flow() {
        // Skill/tool/capability installs must NOT hit the OS package flow — they
        // belong to the marketplace (search_marketplace / install_capability).
        for prompt in [
            "install a web search tool",
            "install the IP Info tool from the marketplace",
            "install a PDF extractor skill",
            "add a zip compression capability",
            "install an OCR plugin",
        ] {
            assert_eq!(
                detect_package_intent(prompt),
                None,
                "capability request must not enter OS package flow: {prompt}"
            );
            assert!(
                refers_to_marketplace_capability(prompt),
                "should be recognized as a marketplace capability: {prompt}"
            );
        }
    }

    #[test]
    fn extract_capability_query_strips_verbs_and_marketplace_filler() {
        assert_eq!(
            extract_capability_query("Install the IP Info tool from the marketplace").as_deref(),
            Some("ip info tool")
        );
        assert_eq!(
            extract_capability_query("search the marketplace for a hash tool").as_deref(),
            Some("hash tool")
        );
        assert_eq!(
            extract_capability_query("add a base64 tool").as_deref(),
            Some("base64 tool")
        );
    }
}
