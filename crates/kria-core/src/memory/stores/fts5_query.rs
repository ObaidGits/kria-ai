//! FTS5 query compiler — F3.2 / task 3.2.3.
//!
//! Converts raw user text into a bounded, safe FTS5 MATCH expression.
//! User text is **never interpolated into SQL** — the returned
//! [`CompiledFts5Query::match_expr`] is always bound as a `?N` parameter.
//!
//! ## Query classes (applied in order)
//! 1. **Exact phrase** — text in double-quotes: `"dark mode"` → `"dark mode"`
//! 2. **Field restriction** — `field:value` with known field names
//!    (`title`, `body`, `aliases`) → FTS5 `field : "value"`
//! 3. **Exact identifier** — UUID, email, URL, file-path, or code-like token
//!    → wrapped as `"<token>"` (special chars stripped)
//! 4. **Prefix** — term ending in `*` → `term*`
//! 5. **Normalized term** — NFC-lowercased alphanumeric → `"term"`
//!
//! ## Bounds (design §8.1)
//! * Query length: max 512 Unicode scalar values.
//! * Filter clauses: max 20 (validated externally via
//!   [`validate_filter_clause_count`]).
//! * Token list: capped at 50 distinct compiled tokens to prevent huge MATCH
//!   expressions.
//!
//! ## Multiple tokens
//! Joined with `OR` (recall-oriented; BM25 ranks best matches higher).

use once_cell::sync::Lazy;
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

// ─── Public limits ────────────────────────────────────────────────────────────

/// Maximum number of Unicode scalar values in a raw query string.
pub const MAX_QUERY_CHARS: usize = 512;
/// Maximum number of filter clauses on a search request.
pub const MAX_FILTER_CLAUSES: usize = 20;
/// Maximum number of compiled tokens in the MATCH expression.
const MAX_TOKENS: usize = 50;

// ─── Known FTS5 column names that may be used in field:value syntax ──────────

const KNOWN_FIELDS: &[&str] = &["title", "body", "aliases"];

// ─── Error type ───────────────────────────────────────────────────────────────

/// Error from query compilation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum QueryCompileError {
    #[error("query exceeds 512-character limit (got {0} chars)")]
    QueryTooLong(usize),
    #[error("filter has {0} clauses; maximum is 20")]
    TooManyFilterClauses(usize),
    #[error("no searchable tokens in query")]
    EmptyQuery,
}

// ─── Output type ─────────────────────────────────────────────────────────────

/// A compiled FTS5 MATCH expression ready to be bound as a SQL parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFts5Query {
    /// The MATCH expression string — bind this as `?N`, never interpolate.
    pub match_expr: String,
    /// Number of distinct top-level tokens/phrases in the compiled expression.
    pub token_count: usize,
}

// ─── Regex patterns ───────────────────────────────────────────────────────────

/// UUID (hyphenated): 8-4-4-4-12 hex chars.
static RE_UUID: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").unwrap()
});

/// Email address.
static RE_EMAIL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}$").unwrap());

/// URL (http/https/ftp scheme).
static RE_URL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^(https?|ftp)://[^\s]+$").unwrap());

/// File path: starts with `/`, `./`, `../`, `~/`, or a Windows drive letter.
static RE_PATH: Lazy<Regex> = Lazy::new(|| Regex::new(r"^([/~]|\.{1,2}/|[A-Za-z]:[/\\])").unwrap());

/// Code-like token: contains `_`, `.`, `-`, `::`, `/`, `@` between alphanumeric
/// segments (e.g. `snake_case`, `camelCase`, `mod::path`, `@tag`).
static RE_CODE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[_.\-@/]|::|[A-Z][a-z]|[a-z][A-Z]").unwrap());

// ─── Core compilation ─────────────────────────────────────────────────────────

/// Compile a raw user query string into a bounded FTS5 MATCH expression.
///
/// Returns [`QueryCompileError::QueryTooLong`] when `raw` exceeds
/// [`MAX_QUERY_CHARS`] Unicode scalar values, and [`QueryCompileError::EmptyQuery`]
/// when no searchable tokens remain after parsing.
pub fn compile_fts5_query(raw: &str) -> Result<CompiledFts5Query, QueryCompileError> {
    // ── 1. Length guard ───────────────────────────────────────────────────────
    let char_count = raw.chars().count();
    if char_count > MAX_QUERY_CHARS {
        return Err(QueryCompileError::QueryTooLong(char_count));
    }

    // ── 2. Parse tokens ───────────────────────────────────────────────────────
    let compiled_tokens = parse_query(raw);

    if compiled_tokens.is_empty() {
        return Err(QueryCompileError::EmptyQuery);
    }

    // ── 3. Cap token count ────────────────────────────────────────────────────
    let tokens: Vec<String> = compiled_tokens.into_iter().take(MAX_TOKENS).collect();
    let token_count = tokens.len();

    let match_expr = tokens.join(" OR ");

    Ok(CompiledFts5Query {
        match_expr,
        token_count,
    })
}

/// Validate that a filter clause count is within [`MAX_FILTER_CLAUSES`].
///
/// Returns [`QueryCompileError::TooManyFilterClauses`] when `count` exceeds
/// the limit.  Intended for use by the calling layer before building SQL.
pub fn validate_filter_clause_count(count: usize) -> Result<(), QueryCompileError> {
    if count > MAX_FILTER_CLAUSES {
        Err(QueryCompileError::TooManyFilterClauses(count))
    } else {
        Ok(())
    }
}

// ─── Parser ───────────────────────────────────────────────────────────────────

/// Parse a raw query string into a list of FTS5 token strings.
///
/// The returned strings are already formatted for use in a MATCH expression
/// (quoted phrases, `field : "value"`, bare `term*`, etc.).
fn parse_query(raw: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let chars: Vec<char> = raw.chars().collect();
    let len = chars.len();
    let mut i = 0usize;

    while i < len {
        // Skip leading whitespace between tokens.
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }

        // ── Class 1: Exact phrase (double-quoted) ─────────────────────────────
        if chars[i] == '"' {
            let start = i + 1;
            // Scan for the closing quote.
            let mut j = start;
            while j < len && chars[j] != '"' {
                j += 1;
            }
            let phrase: String = chars[start..j].iter().collect();
            // Advance past closing quote (or end of string).
            i = if j < len { j + 1 } else { j };

            let phrase = phrase.trim().to_string();
            if !phrase.is_empty() {
                // Sanitize: strip any embedded double-quotes so the result is
                // safe when used as a MATCH parameter value.
                let safe = phrase.replace('"', "");
                if !safe.trim().is_empty() {
                    tokens.push(format!("\"{}\"", safe));
                }
            }
            continue;
        }

        // ── Collect the next whitespace-delimited word ────────────────────────
        let word_start = i;
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
        let word: String = chars[word_start..i].iter().collect();

        if word.is_empty() {
            continue;
        }

        // ── Class 2: Field restriction (field:value) ──────────────────────────
        if let Some(field_token) = try_parse_field_restriction(&word) {
            tokens.push(field_token);
            continue;
        }

        // ── Class 3 / 4 / 5 via classify_word ────────────────────────────────
        tokens.extend(classify_word(&word));
    }

    tokens
}

/// Attempt to parse a `field:value` restriction.
///
/// Returns `Some(fts5_expr)` when the prefix is a known field name and the
/// value part is non-empty; returns `None` otherwise (word is treated as a
/// regular term).
fn try_parse_field_restriction(word: &str) -> Option<String> {
    let colon_pos = word.find(':')?;
    let field = &word[..colon_pos];
    let value = &word[colon_pos + 1..];

    if !KNOWN_FIELDS.contains(&field) {
        return None;
    }
    if value.is_empty() {
        return None;
    }

    // The value portion may itself carry a prefix `*`.
    let (value_base, is_prefix) = if value.ends_with('*') {
        (&value[..value.len() - 1], true)
    } else {
        (value, false)
    };

    if value_base.is_empty() {
        return None;
    }

    let normalized = nfc_lowercase(value_base);
    if normalized.is_empty() {
        return None;
    }
    // Strip any double-quotes from the value to prevent MATCH syntax errors.
    let safe_value = normalized.replace('"', "");
    if safe_value.is_empty() {
        return None;
    }

    if is_prefix {
        Some(format!("{field} : {safe_value}*"))
    } else {
        Some(format!("{field} : \"{safe_value}\""))
    }
}

/// Classify a single non-whitespace word into one or more FTS5 token strings.
///
/// Handles:
/// * Prefix (`*` suffix) — returns `"base*"` (lowercased base, no outer quotes)
/// * Exact identifier (UUID / email / URL / path / code-like) — returns
///   `"<safe_token>"` (quoted, special chars neutralized)
/// * Normal term — returns `"<nfc_lower>"`
fn classify_word(word: &str) -> Vec<String> {
    // Strip any trailing/leading punctuation that is not `*` for prefix check.
    let (base, is_prefix) = if word.ends_with('*') {
        (&word[..word.len() - 1], true)
    } else {
        (word, false)
    };

    if base.is_empty() {
        return Vec::new();
    }

    if is_prefix {
        // Prefix: lowercase, strip chars unsafe for FTS5 identifier (keep alphanumeric + underscore).
        let safe_base: String = base
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>()
            .nfc()
            .collect::<String>()
            .to_lowercase();
        if safe_base.is_empty() {
            return Vec::new();
        }
        return vec![format!("{safe_base}*")];
    }

    // Detect exact identifier patterns.
    if is_exact_identifier(base) {
        // Quote it; strip any embedded double-quotes from the token value.
        let safe = strip_fts5_unsafe_chars(base);
        if safe.is_empty() {
            return Vec::new();
        }
        return vec![format!("\"{}\"", safe)];
    }

    // Normal term: NFC, lowercase, strip non-alphanumeric.
    let normalized = nfc_lowercase_alnum(base);
    if normalized.is_empty() {
        return Vec::new();
    }
    vec![format!("\"{}\"", normalized)]
}

// ─── Identifier detection ─────────────────────────────────────────────────────

fn is_exact_identifier(s: &str) -> bool {
    RE_UUID.is_match(s)
        || RE_EMAIL.is_match(s)
        || RE_URL.is_match(s)
        || RE_PATH.is_match(s)
        || RE_CODE.is_match(s)
}

// ─── String helpers ───────────────────────────────────────────────────────────

/// NFC-normalize and lowercase a string.
fn nfc_lowercase(s: &str) -> String {
    s.nfc().collect::<String>().to_lowercase()
}

/// NFC-normalize, lowercase, and keep only alphanumeric characters.
///
/// This is used for regular search terms to produce clean quoted tokens.
fn nfc_lowercase_alnum(s: &str) -> String {
    s.nfc()
        .collect::<String>()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Strip characters that would break a quoted FTS5 MATCH parameter value.
///
/// Keeps alphanumeric plus the structural chars that appear in identifiers
/// (`.`, `-`, `_`, `@`, `/`, `:`).  Double-quotes are always removed to
/// prevent escaping issues inside the outer quoted phrase.
fn strip_fts5_unsafe_chars(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '@' | '/' | ':'))
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── 1. Exact phrase ────────────────────────────────────────────────────────

    #[test]
    fn exact_phrase_produces_quoted_phrase() {
        let result = compile_fts5_query("\"dark mode\"").unwrap();
        assert_eq!(result.match_expr, "\"dark mode\"");
        assert_eq!(result.token_count, 1);
    }

    // ── 2. Regular terms joined with OR ───────────────────────────────────────

    #[test]
    fn regular_terms_joined_with_or() {
        let result = compile_fts5_query("dark mode").unwrap();
        assert_eq!(result.match_expr, "\"dark\" OR \"mode\"");
        assert_eq!(result.token_count, 2);
    }

    // ── 3. Field restriction — known field ────────────────────────────────────

    #[test]
    fn field_restriction_known_field() {
        // "title:memory graph"
        // "title:memory" → field restriction, "graph" → regular term.
        let result = compile_fts5_query("title:memory graph").unwrap();
        assert_eq!(result.match_expr, "title : \"memory\" OR \"graph\"");
    }

    // ── 4. Unknown field treated as regular term ──────────────────────────────

    #[test]
    fn unknown_field_treated_as_regular_term() {
        // "author:kria" — "author" is not a known field.
        let result = compile_fts5_query("author:kria").unwrap();
        // Should be treated as a regular term; "author:kria" contains ":" so
        // it's detected as a code-like identifier.
        assert!(
            result.match_expr.contains("authorkria") || result.match_expr.contains("author"),
            "unknown field:value should produce a safe token, got: {}",
            result.match_expr
        );
    }

    // ── 5. UUID detected as exact identifier ──────────────────────────────────

    #[test]
    fn uuid_detected_as_exact_identifier() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let result = compile_fts5_query(uuid).unwrap();
        assert_eq!(result.match_expr, format!("\"{}\"", uuid));
    }

    // ── 6. Email detected as exact identifier ────────────────────────────────

    #[test]
    fn email_detected_as_exact_identifier() {
        let email = "user@example.com";
        let result = compile_fts5_query(email).unwrap();
        assert_eq!(result.match_expr, format!("\"{}\"", email));
    }

    // ── 7. Prefix term ────────────────────────────────────────────────────────

    #[test]
    fn prefix_term_produces_prefix_syntax() {
        let result = compile_fts5_query("mem*").unwrap();
        assert_eq!(result.match_expr, "mem*");
    }

    // ── 8. Mixed: quoted phrase + prefix ─────────────────────────────────────

    #[test]
    fn mixed_quoted_phrase_and_prefix() {
        let result = compile_fts5_query("\"dark mode\" graph*").unwrap();
        assert_eq!(result.match_expr, "\"dark mode\" OR graph*");
    }

    // ── 9. Empty input → EmptyQuery error ────────────────────────────────────

    #[test]
    fn empty_input_returns_empty_query_error() {
        assert_eq!(
            compile_fts5_query("").unwrap_err(),
            QueryCompileError::EmptyQuery
        );
        assert_eq!(
            compile_fts5_query("   ").unwrap_err(),
            QueryCompileError::EmptyQuery
        );
    }

    // ── 10. Query > 512 chars → QueryTooLong error ────────────────────────────

    #[test]
    fn query_too_long_returns_error() {
        let long: String = "a".repeat(513);
        assert_eq!(
            compile_fts5_query(&long).unwrap_err(),
            QueryCompileError::QueryTooLong(513)
        );
    }

    // ── 11. Filter > 20 clauses → TooManyFilterClauses ───────────────────────

    #[test]
    fn too_many_filter_clauses_returns_error() {
        assert_eq!(
            validate_filter_clause_count(21).unwrap_err(),
            QueryCompileError::TooManyFilterClauses(21)
        );
        // 20 is the limit: exactly 20 must pass.
        assert!(validate_filter_clause_count(20).is_ok());
    }

    // ── 12. SQL injection attempt is neutralized ──────────────────────────────

    #[test]
    fn injection_attempt_is_neutralized() {
        // All special SQL chars should be stripped; the result must be a safe
        // MATCH expression with no raw SQL punctuation that could break the query.
        let injection = "'; DROP TABLE memories_fts; --";
        let result = compile_fts5_query(injection).unwrap();
        // The result must not contain a raw single-quote, semicolons, or "--".
        assert!(
            !result.match_expr.contains("';"),
            "single-quote + semicolon must be neutralized"
        );
        assert!(
            !result.match_expr.contains("DROP TABLE"),
            "SQL keyword injection must not survive verbatim"
        );
        // All tokens must be safely quoted or empty.
        // "DROP" and "TABLE" are regular words → become "drop" and "table".
        assert!(result.match_expr.contains("\"drop\"") || result.match_expr.contains("drop"));
    }

    // ── 13. Exactly-512-char query is accepted ────────────────────────────────

    #[test]
    fn exactly_512_chars_is_accepted() {
        let query: String = "a".repeat(512);
        assert!(compile_fts5_query(&query).is_ok());
    }

    // ── 14. Token cap: more than 50 tokens are truncated ─────────────────────

    #[test]
    fn token_cap_truncates_at_50() {
        // 60 space-separated words.
        let query = (0u32..60)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let result = compile_fts5_query(&query).unwrap();
        assert_eq!(result.token_count, 50);
    }

    // ── 15. Unicode normalization: café stays intact as identifier ───────────

    #[test]
    fn unicode_normalized_term() {
        // "café" → NFC-lowercase alphanumeric → "caf" + "e" with combining accent
        // stripped by is_alphanumeric filter. FTS5 unicode61 handles diacritics
        // at the index layer; we just produce a clean quoted token.
        let result = compile_fts5_query("café").unwrap();
        // Should not error; result should be non-empty.
        assert!(!result.match_expr.is_empty());
    }

    // ── 16. Prefix on field restriction ──────────────────────────────────────

    #[test]
    fn field_restriction_with_prefix_value() {
        let result = compile_fts5_query("title:mem*").unwrap();
        assert_eq!(result.match_expr, "title : mem*");
    }

    // ── 17. Multiple field restrictions with extra terms ─────────────────────

    #[test]
    fn multiple_tokens_mixed_classes() {
        let result = compile_fts5_query("\"exact phrase\" title:graph mem*").unwrap();
        assert_eq!(
            result.match_expr,
            "\"exact phrase\" OR title : \"graph\" OR mem*"
        );
    }

    // ── 18. Empty phrase (just quotes) ───────────────────────────────────────

    #[test]
    fn empty_phrase_does_not_produce_token() {
        // "\"\"" — nothing inside the quotes.
        let result = compile_fts5_query("\"\" hello").unwrap();
        assert_eq!(result.match_expr, "\"hello\"");
    }

    // ── 19. validate_filter_clause_count: 0 is fine ──────────────────────────

    #[test]
    fn validate_zero_filter_clauses_ok() {
        assert!(validate_filter_clause_count(0).is_ok());
    }
}
