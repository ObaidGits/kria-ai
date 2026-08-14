//! `SchemaEntityIndex` — resolve a free-text message to candidate config fields
//! (settings-nl-control Task 6). Driven ENTIRELY by the schema (`FieldMeta`
//! synonyms + section/field names + valid_values) — never per-prompt keyword
//! branches (Req 3.2). Built once and cached (Req 12.1); scales to 500+ fields.
//!
//! Wave 2 ships the tier-A lexical matcher (phrase-substring + token overlap),
//! which is always available (offline/cold-start, F3) and must pass the golden
//! set's field-resolution cases. A tier-B embedding matcher can layer on later
//! behind the same `resolve` API without changing callers.

use crate::config::schema;

/// One indexed field with its matchable phrases/tokens.
#[derive(Clone, Debug)]
struct FieldEntry {
    section: String,
    field: String,
    /// Multi-word synonym phrases (matched as substrings — strong signal).
    phrases: Vec<String>,
    /// Distinctive single tokens (section/field/synonym words) for overlap scoring.
    tokens: Vec<String>,
    prompt_changeable: bool,
}

/// A resolved field candidate with a confidence score in `[0,1]`.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldCandidate {
    pub section: String,
    pub field: String,
    pub score: f32,
}

/// Schema-derived, cached index from text → config fields.
pub struct SchemaEntityIndex {
    entries: Vec<FieldEntry>,
}

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "to", "my", "your", "is", "are", "of", "for", "in", "on", "set", "change",
    "make", "turn", "use", "what", "how", "do", "i", "it", "this", "that", "please", "kria", "app",
    "mode", "current", "am", "using", "and", "then",
];

fn normalize(text: &str) -> String {
    text.to_ascii_lowercase()
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

impl SchemaEntityIndex {
    /// Build the index from the current schema. Cache the result (see `AppState`).
    pub fn build() -> Self {
        let mut entries = Vec::new();
        for (section, field) in schema::all_fields() {
            // Skip secrets — never resolve a prompt to a secret field.
            if crate::config::is_secret_field(&section, &field) {
                continue;
            }
            let meta = schema::field_meta(&section, &field);
            let mut phrases: Vec<String> = Vec::new();
            let mut tokens: Vec<String> = Vec::new();

            // Field name words (split snake_case) are distinctive tokens.
            for w in field.split('_') {
                if !w.is_empty() && !STOPWORDS.contains(&w) {
                    tokens.push(w.to_string());
                }
            }
            // Synonyms: multi-word → phrase; also contribute tokens.
            for syn in meta.synonyms {
                let s = syn.to_ascii_lowercase();
                if s.contains(' ') {
                    phrases.push(s.clone());
                }
                for t in tokenize(&s) {
                    if !STOPWORDS.contains(&t.as_str()) {
                        tokens.push(t);
                    }
                }
            }
            tokens.sort();
            tokens.dedup();
            entries.push(FieldEntry {
                section,
                field,
                phrases,
                tokens,
                prompt_changeable: meta.prompt_changeable,
            });
        }
        Self { entries }
    }

    /// Number of indexed (non-secret) fields.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resolve the message to ranked field candidates (highest score first).
    /// Only fields with a positive score are returned.
    pub fn resolve(&self, text: &str) -> Vec<FieldCandidate> {
        let norm = normalize(text);
        let msg_tokens = tokenize(text);
        let mut out: Vec<FieldCandidate> = Vec::new();
        for e in &self.entries {
            let score = self.score_entry(e, &norm, &msg_tokens);
            if score > 0.0 {
                out.push(FieldCandidate {
                    section: e.section.clone(),
                    field: e.field.clone(),
                    score,
                });
            }
        }
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// The single best candidate (if any).
    pub fn best(&self, text: &str) -> Option<FieldCandidate> {
        self.resolve(text).into_iter().next()
    }

    fn score_entry(&self, e: &FieldEntry, norm: &str, msg_tokens: &[String]) -> f32 {
        // Strong: a full synonym phrase appears verbatim.
        let mut best: f32 = 0.0;
        for p in &e.phrases {
            if norm.contains(p.as_str()) {
                // Longer phrase match = stronger + more specific.
                let words = p.split_whitespace().count() as f32;
                best = best.max(0.85 + (words - 1.0) * 0.05).min(1.0);
            }
        }
        // Medium: distinctive token overlap.
        if !e.tokens.is_empty() {
            let hits = e
                .tokens
                .iter()
                .filter(|t| msg_tokens.iter().any(|m| m == *t))
                .count();
            if hits > 0 {
                let overlap = hits as f32 / e.tokens.len() as f32;
                // A single distinctive token hit is a weak signal; scale it.
                let token_score = 0.35 + 0.4 * overlap;
                best = best.max(token_score.min(0.8));
            }
        }
        // Slightly favour prompt-changeable fields (fail-closed fields are rarely
        // the intended target of a settings command).
        if best > 0.0 && !e.prompt_changeable {
            best *= 0.6;
        }
        best
    }

    /// Resolve a VALUE for a field from the message. Delegates to the universal,
    /// type-inferring [`crate::config::nl::value`] engine (settings-nl-intelligence
    /// Wave 1): the field TYPE is inferred from the real config shape and the
    /// enums/values from the schema — so booleans, integers, floats, and enums
    /// (natural spacing/case/alias) all resolve with NO per-field hardcoding.
    /// An unmatched enum value word is returned raw so the handler can reject it
    /// with the allowed-values list (grounded reask).
    pub fn resolve_value(
        &self,
        section: &str,
        field: &str,
        text: &str,
    ) -> Option<serde_json::Value> {
        crate::config::nl::value::extract(section, field, text)
    }
}

#[cfg(test)]
impl SchemaEntityIndex {
    #[allow(dead_code)] // test-only seam
    /// Test-only seam (P8, no-hardcoding): inject a synthetic field so a sibling
    /// property-test module can prove the classifier resolves a brand-new field
    /// purely from its schema phrase/token, with ZERO per-field routing code.
    pub(crate) fn push_synthetic_field(
        &mut self,
        section: &str,
        field: &str,
        phrase: &str,
        token: &str,
    ) {
        self.entries.push(FieldEntry {
            section: section.to_string(),
            field: field.to_string(),
            phrases: vec![phrase.to_ascii_lowercase()],
            tokens: vec![token.to_ascii_lowercase()],
            prompt_changeable: true,
        });
    }
}

#[cfg(test)]
mod tests {
#![allow(dead_code)]  // see note above
    use super::*;

    #[test]
    fn resolves_common_fields_without_keyword_branches() {
        let idx = SchemaEntityIndex::build();
        assert!(!idx.is_empty());
        let theme = idx.best("switch to dark mode").expect("theme");
        assert_eq!(
            (theme.section.as_str(), theme.field.as_str()),
            ("ui", "theme")
        );
        let engine = idx.best("set search engine to duckduckgo").expect("engine");
        assert_eq!(
            (engine.section.as_str(), engine.field.as_str()),
            ("search", "engine")
        );
    }

    #[test]
    fn value_resolution_disambiguates_voice_enabled_over_mode() {
        // "turn off voice": voice.enabled resolves a boolean value; voice.mode does not.
        let idx = SchemaEntityIndex::build();
        let cands = idx.resolve("turn off voice");
        // Score each with value grounding, mimicking the pipeline.
        let scored: Vec<_> = cands
            .iter()
            .map(|c| {
                let v = idx.resolve_value(&c.section, &c.field, "turn off voice");
                let s = if v.is_some() { c.score + 0.4 } else { c.score };
                (c.section.clone(), c.field.clone(), s)
            })
            .collect();
        let best = scored
            .iter()
            .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap())
            .unwrap();
        assert_eq!((best.0.as_str(), best.1.as_str()), ("voice", "enabled"));
    }

    #[test]
    fn secret_fields_are_never_resolved() {
        let idx = SchemaEntityIndex::build();
        for c in idx.resolve("set the cloud api key to sk-123") {
            assert!(
                !crate::config::is_secret_field(&c.section, &c.field),
                "resolved a secret field: {}.{}",
                c.section,
                c.field
            );
        }
    }

    #[test]
    fn value_resolution_is_schema_bounded() {
        let idx = SchemaEntityIndex::build();
        assert_eq!(
            idx.resolve_value("ui", "theme", "change theme to dark"),
            Some(serde_json::json!("dark"))
        );
        // Invalid value returned raw so the handler can reject it with allowed list.
        assert_eq!(
            idx.resolve_value("ui", "theme", "set theme to rainbow"),
            Some(serde_json::json!("rainbow"))
        );
        // Boolean generic mapping.
        assert_eq!(
            idx.resolve_value("voice", "enabled", "turn off voice"),
            Some(serde_json::json!(false))
        );
    }

    #[test]
    fn unrelated_message_resolves_nothing_strong() {
        let idx = SchemaEntityIndex::build();
        // A pure non-settings message should not produce a high-confidence field.
        let best = idx.best("what is the capital of India");
        assert!(best.map(|c| c.score < 0.5).unwrap_or(true));
    }

    // ── P8/Req 12: scaling — resolution stays correct + bounded at 500+ fields ──
    #[test]
    fn scales_to_500_plus_fields_within_latency_budget() {
        // Start from the real schema, then pad with synthetic fields so the index
        // holds 500+ entries. Each synthetic field has a distinctive synonym so we
        // can confirm resolution is still correct (indexed lookup, not linear noise).
        let mut idx = SchemaEntityIndex::build();
        let real = idx.entries.len();
        let target = 500usize.saturating_sub(real).max(500);
        for i in 0..target {
            // Trailing 'x' boundary so no token is a substring prefix of another
            // (e.g. "gizmo2x" is NOT contained in "gizmo250x").
            let token = format!("gizmo{i}x");
            idx.entries.push(FieldEntry {
                section: "synthetic".to_string(),
                field: format!("widget_{i}"),
                phrases: vec![format!("magic {token}")],
                tokens: vec![token],
                prompt_changeable: true,
            });
        }
        assert!(idx.len() >= 500, "index should have 500+ fields");

        // Correctness at scale: a distinctive synthetic phrase resolves to its field.
        let mid = target / 2;
        let best = idx
            .best(&format!("please set magic gizmo{mid}x now"))
            .expect("synthetic field must resolve");
        assert_eq!(best.field, format!("widget_{mid}"));

        // Latency budget: 200 resolutions over the 500+ index stay well bounded.
        let start = std::time::Instant::now();
        for _ in 0..200 {
            let _ = idx.resolve("switch to dark mode and set the search engine");
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 2000,
            "500-field resolve x200 took {elapsed:?} (budget 2s)"
        );
    }
}
