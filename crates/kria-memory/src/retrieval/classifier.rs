//! V2 query classifier for the RRF fusion engine (design §6.2).
//!
//! Assigns one of six deterministic query classes to a raw query string using
//! strict first-match-wins precedence:
//!
//! `Identifier` > `ExactPhrase` > `EntityRelation` > `Temporal` > `ActiveGoal` > `Exploratory`
//!
//! # Design invariants
//! * Deterministic: same input always produces the same output.
//! * Version-stable: rules are frozen at `classifier-v1`. Rule changes bump the version.
//! * `reasons` is always non-empty (`Exploratory` fallback fires if nothing else matches).
//! * No I/O, no randomness, no DB access — pure function.

use crate::retrieval::temporal_strategy::parse_temporal_intent;

// ── Version ──────────────────────────────────────────────────────────────────

/// Classifier version constant (immutable, versioned with the classifier itself).
pub const CLASSIFIER_VERSION: &str = "classifier-v1";

// ── QueryClassV2 ─────────────────────────────────────────────────────────────

/// The v2 query class used by the RRF fusion engine (design §6.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QueryClassV2 {
    /// UUID, path, URL, email, or code-like exact token.
    Identifier,
    /// Quoted phrase or exact-match operator.
    ExactPhrase,
    /// Resolved entity/relation terms.
    EntityRelation,
    /// Parsed instant/range/recency intent.
    Temporal,
    /// Task/resume/next intent with active context.
    ActiveGoal,
    /// Default fallback.
    Exploratory,
}

impl QueryClassV2 {
    /// Machine-readable string label for the class.
    pub fn as_str(&self) -> &'static str {
        match self {
            QueryClassV2::Identifier => "identifier",
            QueryClassV2::ExactPhrase => "exact_phrase",
            QueryClassV2::EntityRelation => "entity_relation",
            QueryClassV2::Temporal => "temporal",
            QueryClassV2::ActiveGoal => "active_goal",
            QueryClassV2::Exploratory => "exploratory",
        }
    }

    /// Fusion profile ID that should be used with this class (design §6.2).
    pub fn profile_id(&self) -> &'static str {
        match self {
            QueryClassV2::Identifier => "rrf-id-v1",
            QueryClassV2::ExactPhrase => "rrf-exact-v1",
            QueryClassV2::EntityRelation => "rrf-graph-v1",
            QueryClassV2::Temporal => "rrf-time-v1",
            QueryClassV2::ActiveGoal => "rrf-goal-v1",
            QueryClassV2::Exploratory => "rrf-general-v1",
        }
    }
}

// ── ClassificationReason ─────────────────────────────────────────────────────

/// One reason why a class was selected.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassificationReason {
    /// Short machine-readable code, e.g. `"uuid_pattern"`, `"quoted_phrase"`.
    pub code: String,
    /// Human-readable explanation, e.g. `"found UUID token: abc-123"`.
    pub detail: String,
}

impl ClassificationReason {
    fn new(code: &str, detail: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            detail: detail.into(),
        }
    }
}

// ── ClassificationResult ─────────────────────────────────────────────────────

/// Result of classifying a query.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassificationResult {
    /// The assigned query class.
    pub class: QueryClassV2,
    /// The classifier version that produced this result.
    pub version: &'static str,
    /// Ordered list of reasons for the classification (non-empty).
    pub reasons: Vec<ClassificationReason>,
    /// The fusion profile ID that should be used with this class (design §6.2).
    pub profile_id: &'static str,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Classify a query text deterministically under `classifier-v1`.
///
/// Returns a [`ClassificationResult`] with the assigned class, version,
/// ordered reasons, and fusion profile ID.  Always returns a result
/// (`Exploratory` is the fallback).
///
/// # Contract
/// * Deterministic: same input always produces the same output.
/// * Version-stable: classification rules are fixed at `classifier-v1`.
///   Rule changes increment the version string.
/// * First-match-wins precedence: `Identifier` > `ExactPhrase` > `EntityRelation`
///   > `Temporal` > `ActiveGoal` > `Exploratory`.
/// * All temporal detection delegates to `temporal_strategy::parse_temporal_intent`.
/// * The returned `reasons` vector is always non-empty.
pub fn classify_query_v2(query: &str) -> ClassificationResult {
    // Precedence 1: Identifier
    if let Some(reasons) = detect_identifier(query) {
        return build_result(QueryClassV2::Identifier, reasons);
    }

    // Precedence 2: ExactPhrase
    if let Some(reasons) = detect_exact_phrase(query) {
        return build_result(QueryClassV2::ExactPhrase, reasons);
    }

    // Precedence 3: EntityRelation
    if let Some(reasons) = detect_entity_relation(query) {
        return build_result(QueryClassV2::EntityRelation, reasons);
    }

    // Precedence 4: Temporal
    if let Some(reasons) = detect_temporal(query) {
        return build_result(QueryClassV2::Temporal, reasons);
    }

    // Precedence 5: ActiveGoal
    if let Some(reasons) = detect_active_goal(query) {
        return build_result(QueryClassV2::ActiveGoal, reasons);
    }

    // Precedence 6: Exploratory (always matches)
    build_result(
        QueryClassV2::Exploratory,
        vec![ClassificationReason::new(
            "default_fallback",
            "no specific class detected — using exploratory fallback",
        )],
    )
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn build_result(class: QueryClassV2, reasons: Vec<ClassificationReason>) -> ClassificationResult {
    ClassificationResult {
        profile_id: class.profile_id(),
        class,
        version: CLASSIFIER_VERSION,
        reasons,
    }
}

// ── 1. Identifier detection ───────────────────────────────────────────────────

/// UUID pattern: `[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}`
fn is_uuid(token: &str) -> bool {
    let b = token.as_bytes();
    if b.len() != 36 {
        return false;
    }
    let dash_positions = [8, 13, 18, 23];
    for &pos in &dash_positions {
        if b[pos] != b'-' {
            return false;
        }
    }
    b.iter().enumerate().all(|(i, &c)| {
        if dash_positions.contains(&i) {
            true
        } else {
            c.is_ascii_hexdigit()
        }
    })
}

/// URL: starts with `http://`, `https://`, `ftp://`, or contains `://`
fn is_url(token: &str) -> bool {
    token.starts_with("http://")
        || token.starts_with("https://")
        || token.starts_with("ftp://")
        || token.contains("://")
}

/// Email: contains `@` and a `.` after the `@`
fn is_email(token: &str) -> bool {
    if let Some(at_pos) = token.find('@') {
        let after_at = &token[at_pos + 1..];
        after_at.contains('.')
    } else {
        false
    }
}

/// File path: starts with `/`, `./`, `~/`, `C:\` or contains `\`
fn is_path(token: &str) -> bool {
    token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("~/")
        || (token.len() >= 3 && token.as_bytes()[1] == b':' && token.as_bytes()[2] == b'\\')
        || token.contains('\\')
}

/// Code-like token: starts with `#`, or Jira-like `[A-Z][A-Z0-9_-]{2,}[-_][0-9]+`
fn is_code_id(token: &str) -> bool {
    if token.starts_with('#') {
        return true;
    }
    // Jira-like: [A-Z][A-Z0-9_-]{2,}[-_][0-9]+
    // e.g. MGR-001, PROJ_42, ABC-123
    let bytes = token.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_uppercase() {
        return false;
    }
    // Find where the uppercase/digit/underscore/dash prefix ends and a separator + digits begin
    // Pattern: one or more uppercase/digit chars (at least 2 more after the first), then [-_], then digits
    let mut i = 1usize;
    // Consume uppercase letters, digits, underscores, dashes (the "project key" part)
    while i < bytes.len()
        && (bytes[i].is_ascii_uppercase()
            || bytes[i].is_ascii_digit()
            || bytes[i] == b'_'
            || bytes[i] == b'-')
    {
        i += 1;
    }
    // We need the project key to be at least 3 chars (first + 2 more) before a separator
    // Actually the spec says [A-Z][A-Z0-9_-]{2,} then [-_][0-9]+
    // Let's match more carefully: find the last separator position
    if i < 2 {
        return false; // project key too short
    }
    // Now check from start: [A-Z][A-Z0-9_-]{2,}[-_][0-9]+
    // The separator is embedded, so let's scan for the rightmost [-_] followed by all digits
    let s = token;
    // Find rightmost '-' or '_' that is followed by all digits
    for sep_pos in (1..s.len()).rev() {
        let sep_byte = bytes[sep_pos];
        if sep_byte == b'-' || sep_byte == b'_' {
            let before = &s[..sep_pos];
            let after = &s[sep_pos + 1..];
            // before must be [A-Z][A-Z0-9_-]{2,} — at least 3 chars total, first uppercase
            // after must be all digits, non-empty
            if before.len() >= 3
                && before.as_bytes()[0].is_ascii_uppercase()
                && before.as_bytes()[1..].iter().all(|&c| {
                    c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_' || c == b'-'
                })
                && !after.is_empty()
                && after.as_bytes().iter().all(|c| c.is_ascii_digit())
            {
                return true;
            }
        }
    }
    false
}

fn detect_identifier(query: &str) -> Option<Vec<ClassificationReason>> {
    for token in query.split_whitespace() {
        // Strip surrounding quote characters and trailing punctuation that may wrap a token.
        let token = token.trim_matches('"').trim_end_matches(|c: char| {
            !c.is_alphanumeric()
                && c != '-'
                && c != '_'
                && c != '.'
                && c != '@'
                && c != '/'
                && c != '\\'
                && c != ':'
        });

        if is_uuid(token) {
            return Some(vec![ClassificationReason::new(
                "uuid_pattern",
                format!("found UUID token: {token}"),
            )]);
        }
        if is_url(token) {
            return Some(vec![ClassificationReason::new(
                "url_pattern",
                format!("found URL token: {token}"),
            )]);
        }
        if is_email(token) {
            return Some(vec![ClassificationReason::new(
                "email_pattern",
                format!("found email token: {token}"),
            )]);
        }
        if is_path(token) {
            return Some(vec![ClassificationReason::new(
                "path_pattern",
                format!("found path token: {token}"),
            )]);
        }
        if is_code_id(token) {
            return Some(vec![ClassificationReason::new(
                "code_id_pattern",
                format!("found code-like ID token: {token}"),
            )]);
        }
    }
    None
}

// ── 2. ExactPhrase detection ──────────────────────────────────────────────────

fn detect_exact_phrase(query: &str) -> Option<Vec<ClassificationReason>> {
    // Double-quoted substring: query contains `"..."` (at least one char between quotes)
    let bytes = query.as_bytes();
    let mut in_quote = false;
    for &b in bytes {
        if b == b'"' {
            if in_quote {
                // Found closing quote — there was a quoted substring
                return Some(vec![ClassificationReason::new(
                    "double_quoted_phrase",
                    "query contains a double-quoted phrase",
                )]);
            }
            in_quote = true;
        }
    }

    // Explicit exact-match operator: starts with `=` or `exact:` prefix
    let trimmed = query.trim();
    if trimmed.starts_with('=') {
        return Some(vec![ClassificationReason::new(
            "exact_operator",
            "query starts with '=' exact-match operator",
        )]);
    }
    if trimmed.to_ascii_lowercase().starts_with("exact:") {
        return Some(vec![ClassificationReason::new(
            "exact_operator",
            "query starts with 'exact:' exact-match operator",
        )]);
    }

    None
}

// ── 3. EntityRelation detection ───────────────────────────────────────────────

const RELATION_KEYWORDS: &[&str] = &[
    " knows ",
    " owns ",
    " created ",
    " is a ",
    " part of ",
    " related to ",
    " belongs to ",
    " authored ",
    " worked at ",
    " member of ",
    " connected to ",
    " linked to ",
    " derived from ",
    " superseded by ",
    " mentions ",
    " supports ",
    " contradicts ",
];

const RELATIONSHIP_QUERIES: &[&str] = &[" relationship", " relation", " edge", "who is related"];

fn detect_entity_relation(query: &str) -> Option<Vec<ClassificationReason>> {
    let lower = query.to_ascii_lowercase();

    // Check relation keywords
    for &kw in RELATION_KEYWORDS {
        if lower.contains(kw) {
            return Some(vec![ClassificationReason::new(
                "relation_keyword",
                format!("found relation keyword: '{}'", kw.trim()),
            )]);
        }
    }

    // Check relationship query patterns
    for &pat in RELATIONSHIP_QUERIES {
        if lower.contains(pat) {
            return Some(vec![ClassificationReason::new(
                "relationship_query",
                format!("query asks about relationships (pattern: '{pat}')"),
            )]);
        }
    }

    // "how are .* connected" regex-like check (simple substring heuristic)
    if lower.contains("how are") && lower.contains("connected") {
        return Some(vec![ClassificationReason::new(
            "relationship_query",
            "query asks how entities are connected",
        )]);
    }

    // Two or more capitalized words that are NOT the first word of the query
    let words: Vec<&str> = query.split_whitespace().collect();
    let capitalized_non_first: Vec<&str> = words
        .iter()
        .skip(1) // skip the first word
        .filter(|w| {
            let first_char = w.chars().next();
            first_char.map(|c| c.is_uppercase()).unwrap_or(false) && w.len() >= 2
            // Exclude all-caps abbreviations that look like code IDs (handled by Identifier)
            // We still want named entities like "Alice", "Bob", "Memory"
        })
        .copied()
        .collect();

    if capitalized_non_first.len() >= 2 {
        return Some(vec![ClassificationReason::new(
            "capitalized_entity_terms",
            format!(
                "found {} capitalized non-initial words: {}",
                capitalized_non_first.len(),
                capitalized_non_first.join(", ")
            ),
        )]);
    }

    None
}

// ── 4. Temporal detection ─────────────────────────────────────────────────────

/// Embedded recency patterns that `parse_temporal_intent` does not match when
/// they appear mid-sentence (it only matches them as standalone queries via
/// `strip_prefix`). We scan the lowercased query for these substrings ourselves.
const EMBEDDED_RECENCY_PATTERNS: &[&str] = &[
    "last week",
    "last month",
    "last year",
    "last day",
    "this week",
    "this month",
    "this year",
    "last hour",
    "yesterday",
    "last 7 days",
    "last 30 days",
    "last 90 days",
];

fn detect_temporal(query: &str) -> Option<Vec<ClassificationReason>> {
    // Primary: delegate to the canonical parser (handles standalone queries and
    // exact-prefix patterns).
    if let Some(intent) = parse_temporal_intent(query) {
        use crate::retrieval::temporal_strategy::TemporalIntent;
        let detail = match &intent {
            TemporalIntent::Instant(dt) => format!("detected Instant temporal intent: {dt}"),
            TemporalIntent::Range(from, to) => {
                format!("detected Range temporal intent: {from} to {to}")
            }
            TemporalIntent::Recency { max_age_days } => {
                format!("detected Recency temporal intent: last {max_age_days} days")
            }
        };
        return Some(vec![ClassificationReason::new(
            "temporal_intent_detected",
            detail,
        )]);
    }

    // Fallback: match embedded recency keywords inside longer sentences that the
    // canonical parser misses because it uses `strip_prefix` rather than substring
    // search.
    let lower = query.to_ascii_lowercase();
    for &pat in EMBEDDED_RECENCY_PATTERNS {
        if lower.contains(pat) {
            return Some(vec![ClassificationReason::new(
                "temporal_intent_detected",
                format!("detected embedded recency pattern: '{pat}'"),
            )]);
        }
    }

    None
}

// ── 5. ActiveGoal detection ───────────────────────────────────────────────────

/// Signals for resume/continuation intent.
const RESUME_SIGNALS: &[&str] = &[
    "resume",
    "continue",
    "next step",
    "where was i",
    "what was i",
    "in progress",
    "working on",
    "ongoing",
    "pick up",
    "get back to",
];

/// Signals for goal/task intent.
const GOAL_TASK_SIGNALS: &[&str] = &["my goal", "current task", "task ", "complete ", "finish "];

fn detect_active_goal(query: &str) -> Option<Vec<ClassificationReason>> {
    let lower = query.to_ascii_lowercase();

    for &signal in RESUME_SIGNALS {
        if lower.contains(signal) {
            return Some(vec![ClassificationReason::new(
                "resume_intent",
                format!("found resume/continuation signal: '{signal}'"),
            )]);
        }
    }

    for &signal in GOAL_TASK_SIGNALS {
        // "task " and "complete " and "finish " match at start or with space prefix
        if lower.contains(signal) || lower.starts_with(signal.trim()) {
            return Some(vec![ClassificationReason::new(
                "goal_task_intent",
                format!("found goal/task signal: '{signal}'"),
            )]);
        }
    }

    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Identifier tests ──────────────────────────────────────────────────────

    #[test]
    fn uuid_token_classified_as_identifier() {
        let result = classify_query_v2("show me abc12345-0000-0000-0000-000000000001");
        assert_eq!(result.class, QueryClassV2::Identifier);
        assert_eq!(result.reasons[0].code, "uuid_pattern");
    }

    #[test]
    fn url_token_classified_as_identifier() {
        let result = classify_query_v2("https://example.com/path");
        assert_eq!(result.class, QueryClassV2::Identifier);
        assert_eq!(result.reasons[0].code, "url_pattern");
    }

    #[test]
    fn email_token_classified_as_identifier() {
        let result = classify_query_v2("contact user@example.com");
        assert_eq!(result.class, QueryClassV2::Identifier);
        assert_eq!(result.reasons[0].code, "email_pattern");
    }

    #[test]
    fn jira_id_classified_as_identifier() {
        let result = classify_query_v2("tell me about MGR-001");
        assert_eq!(result.class, QueryClassV2::Identifier);
        assert_eq!(result.reasons[0].code, "code_id_pattern");
    }

    #[test]
    fn path_token_classified_as_identifier() {
        let result = classify_query_v2("open /home/user/file.txt");
        assert_eq!(result.class, QueryClassV2::Identifier);
        assert_eq!(result.reasons[0].code, "path_pattern");
    }

    // ── ExactPhrase tests ─────────────────────────────────────────────────────

    #[test]
    fn quoted_phrase_classified_as_exact_phrase() {
        // The outer string uses regular quotes; inner query has embedded double-quotes
        let result = classify_query_v2(r#"search for "memory graph""#);
        assert_eq!(result.class, QueryClassV2::ExactPhrase);
        assert_eq!(result.reasons[0].code, "double_quoted_phrase");
    }

    #[test]
    fn exact_operator_classified_as_exact_phrase() {
        let result = classify_query_v2("exact:memory graph");
        assert_eq!(result.class, QueryClassV2::ExactPhrase);
        assert_eq!(result.reasons[0].code, "exact_operator");
    }

    // ── Precedence tests ──────────────────────────────────────────────────────

    #[test]
    fn identifier_beats_exact_phrase() {
        // UUID inside a double-quoted string → Identifier wins, not ExactPhrase
        let result = classify_query_v2(r#""abc12345-0000-0000-0000-000000000001""#);
        assert_eq!(result.class, QueryClassV2::Identifier);
    }

    #[test]
    fn exact_phrase_beats_entity_relation() {
        // Quoted phrase with capitalized entity words → ExactPhrase wins
        let result = classify_query_v2(r#""Alice knows Bob""#);
        assert_eq!(result.class, QueryClassV2::ExactPhrase);
    }

    #[test]
    fn temporal_beats_exploratory() {
        let result = classify_query_v2("what happened last week?");
        assert_eq!(result.class, QueryClassV2::Temporal);
    }

    // ── EntityRelation tests ──────────────────────────────────────────────────

    #[test]
    fn capitalized_terms_classified_as_entity_relation() {
        let result = classify_query_v2("what does Alice know about Bob");
        assert_eq!(result.class, QueryClassV2::EntityRelation);
        assert_eq!(result.reasons[0].code, "capitalized_entity_terms");
    }

    #[test]
    fn relation_keyword_classified_as_entity_relation() {
        let result = classify_query_v2("who created the memory system");
        assert_eq!(result.class, QueryClassV2::EntityRelation);
    }

    // ── Temporal tests ────────────────────────────────────────────────────────

    #[test]
    fn date_query_classified_as_temporal() {
        let result = classify_query_v2("2024-01-15");
        assert_eq!(result.class, QueryClassV2::Temporal);
    }

    #[test]
    fn recency_query_classified_as_temporal() {
        let result = classify_query_v2("last 7 days");
        assert_eq!(result.class, QueryClassV2::Temporal);
    }

    #[test]
    fn non_temporal_not_classified_as_temporal() {
        let result = classify_query_v2("tell me about dogs");
        assert_ne!(result.class, QueryClassV2::Temporal);
    }

    // ── ActiveGoal tests ──────────────────────────────────────────────────────

    #[test]
    fn resume_query_classified_as_active_goal() {
        let result = classify_query_v2("resume my work");
        assert_eq!(result.class, QueryClassV2::ActiveGoal);
        assert_eq!(result.reasons[0].code, "resume_intent");
    }

    #[test]
    fn next_step_classified_as_active_goal() {
        let result = classify_query_v2("what is the next step");
        assert_eq!(result.class, QueryClassV2::ActiveGoal);
    }

    #[test]
    fn task_classified_as_active_goal() {
        let result = classify_query_v2("task memory redesign");
        assert_eq!(result.class, QueryClassV2::ActiveGoal);
    }

    // ── Exploratory tests ─────────────────────────────────────────────────────

    #[test]
    fn plain_question_classified_as_exploratory() {
        let result = classify_query_v2("tell me about the memory system");
        assert_eq!(result.class, QueryClassV2::Exploratory);
    }

    #[test]
    fn empty_query_classified_as_exploratory() {
        let result = classify_query_v2("");
        assert_eq!(result.class, QueryClassV2::Exploratory);
    }

    // ── Version and profile tests ─────────────────────────────────────────────

    #[test]
    fn classifier_version_is_correct() {
        assert_eq!(classify_query_v2("anything").version, CLASSIFIER_VERSION);
        assert_eq!(CLASSIFIER_VERSION, "classifier-v1");
    }

    #[test]
    fn profile_id_matches_class() {
        assert_eq!(QueryClassV2::Identifier.profile_id(), "rrf-id-v1");
        assert_eq!(QueryClassV2::ExactPhrase.profile_id(), "rrf-exact-v1");
        assert_eq!(QueryClassV2::EntityRelation.profile_id(), "rrf-graph-v1");
        assert_eq!(QueryClassV2::Temporal.profile_id(), "rrf-time-v1");
        assert_eq!(QueryClassV2::ActiveGoal.profile_id(), "rrf-goal-v1");
        assert_eq!(QueryClassV2::Exploratory.profile_id(), "rrf-general-v1");
    }

    #[test]
    fn reasons_always_non_empty() {
        for query in &[
            "anything",
            "",
            "2024-01-15",
            "user@example.com",
            r#""quoted""#,
        ] {
            let result = classify_query_v2(query);
            assert!(
                !result.reasons.is_empty(),
                "reasons empty for query: {query:?}"
            );
        }
    }

    #[test]
    fn all_classes_have_string_representations() {
        assert_eq!(QueryClassV2::Identifier.as_str(), "identifier");
        assert_eq!(QueryClassV2::ExactPhrase.as_str(), "exact_phrase");
        assert_eq!(QueryClassV2::EntityRelation.as_str(), "entity_relation");
        assert_eq!(QueryClassV2::Temporal.as_str(), "temporal");
        assert_eq!(QueryClassV2::ActiveGoal.as_str(), "active_goal");
        assert_eq!(QueryClassV2::Exploratory.as_str(), "exploratory");
    }
}
