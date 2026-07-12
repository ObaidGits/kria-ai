//! `SettingsIntentPipeline` — classify a user message into a `SettingsDecision`
//! (settings-nl-control Task 8). Intent, NOT keywords: field/value resolution is
//! entirely schema-driven (`SchemaEntityIndex`); Configuration-vs-Conversation is
//! resolved with subject/topic signals (`ConversationContext`) + a scored
//! confidence with tunable thresholds (design Wave 5 F8). Fails toward
//! conversation (Req 2.6). Tier-A (lexical, offline) is implemented here; an
//! embedder/LLM refinement layer can slot in behind the same API (F2/F3).

use std::sync::Arc;

use crate::config::nl::conversation::{ConversationContext, SubjectSignal};
use crate::config::nl::entity_index::{FieldCandidate, SchemaEntityIndex};
use crate::config::nl::handler::InfoQuery;
use crate::config::prompt::Scope;

/// The classifier's decision for a message.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsDecision {
    Change {
        section: String,
        field: String,
        value: Option<serde_json::Value>,
        scope: Scope,
    },
    ReadBack {
        section: String,
        field: String,
    },
    /// A read-only "answer from the system" query (catalog/help/explain/recent).
    Info(InfoQuery),
    Undo,
    Clarify {
        question: String,
    },
    NotSettings,
}

/// Tunable scoring thresholds (design Wave 5 F8). Defaults calibrated on the
/// golden set; exposed so they can be tuned without code changes.
#[derive(Clone, Copy, Debug)]
pub struct IntentThresholds {
    pub act: f32,
    pub clarify: f32,
    pub domain_floor: f32,
}
impl Default for IntentThresholds {
    fn default() -> Self {
        Self {
            act: 0.72,
            clarify: 0.45,
            domain_floor: 0.40,
        }
    }
}

/// Diagnostic trace of a decision (design C6 / observability). Records the
/// independent evidence signals + fused confidence so every route is explainable.
#[derive(Clone, Debug, Default)]
pub struct SettingsIntentTrace {
    pub best_field: Option<(String, String)>,
    pub entity_score: f32,
    pub value_grounded: bool,
    pub is_question: bool,
    pub is_imperative: bool,
    pub self_ref: bool,
    pub subject: Option<&'static str>,
    /// Semantic (or lexical-fallback) conversation topic-affinity ∈ [0,1].
    pub conversation_topic: f32,
    /// Memory topic-affinity ∈ [0,1] (0 when no memory source).
    pub memory_topic: f32,
    /// Whether tier-B embeddings participated in this decision.
    pub embeddings_used: bool,
    pub confidence: f32,
    pub decision: &'static str,
}

pub struct SettingsIntentPipeline {
    entity_index: Arc<SchemaEntityIndex>,
    thresholds: IntentThresholds,
    deps: crate::config::nl::evidence::EvidenceDeps,
}

// Generic verb/marker sets. These classify INTENT KIND (not fields/values), so
// they are domain-general, not per-setting hardcoding.
const UNDO_MARKERS: &[&str] = &[
    "undo",
    "revert",
    "roll back",
    "rollback",
    "change it back",
    "change that back",
    "put it back",
    "restore previous",
    "restore the previous",
    "go back to previous",
    "go back to the previous",
    "undo that",
    "undo the",
    "revert the",
    "revert previous",
    "revert to previous",
];
const QUESTION_STARTS: &[&str] = &[
    "what",
    "which",
    "how",
    "is",
    "are",
    "do",
    "does",
    "can you tell",
    "tell me",
];
const SELF_REF_READBACK: &[&str] = &[
    "my ",
    "current",
    "currently",
    "am i using",
    "i'm using",
    "i am using",
    "do i have",
    "i use",
    "i'm on",
    "i am on",
    "set to",
];
const IMPERATIVE_VERBS: &[&str] = &[
    "set",
    "change",
    "switch",
    "turn",
    "enable",
    "disable",
    "use",
    "make",
    "increase",
    "decrease",
    "raise",
    "lower",
    "activate",
    "deactivate",
    "select",
    "update",
    "configure",
    "put",
    "toggle",
    // NOTE: content/task verbs (generate, create, write, draw, build) are
    // intentionally EXCLUDED — a content request ("generate an image of a cat")
    // must never be treated as a settings command (no-interference, Req 6.2).
    // Turn-scoped image routing ("generate this using local AI") is still caught
    // by the value-grounded implicit path (it resolves a concrete image_mode value).
];
/// Negation/opinion markers — a grounded value inside a complaint ("I don't like
/// dark mode") is a statement, NOT a command. Guards the implicit-command path.
const COMPLAINT_MARKERS: &[&str] = &[
    "don't",
    "do not",
    "n't like",
    "i prefer",
    "i hate",
    "i like ",
    "not a fan",
];
/// Desire phrasings (change request without an imperative verb). Only act when a
/// settings field + value also resolve, so non-settings desires pass through.
const DESIRE_MARKERS: &[&str] = &[
    "i want ",
    "i'd like",
    "id like",
    "i would like",
    "gimme ",
    "give me ",
];
/// Content-authoring lead verbs. A message that STARTS by asking to author
/// content (write/generate/draw/…) is a content-generation request, never a
/// settings mutation — even when it names a settings word ("write code to change
/// a theme", "draw a dark poster"). No-interference guard (Req 6.2). Generic verb
/// class, not per-setting. The turn-scoped image-routing phrasing
/// ("generate this using local/cloud") is exempted below so image-mode routing
/// still works.
const CONTENT_LEAD_VERBS: &[&str] = &[
    "write ",
    "generate ",
    "create ",
    "draw ",
    "compose ",
    "build ",
    "design ",
    "paint ",
    "render ",
    "code ",
];
/// Copular tokens marking a DECLARATIVE statement/opinion ("dark mode is ugly").
/// A short setting phrase inside a statement is a comment, not a command.
const STATEMENT_COPULAS: &[&str] = &[
    " is ", " are ", " was ", " were ", " isn't ", " aren't ", " looks ", " seems ",
];
const TEMP_CUES: &[&str] = &[
    "for this one",
    "just this",
    "this time",
    "temporarily",
    "for now",
    "for this image",
    "for the next",
    "just for this",
    "this once",
];

/// Whole-word / whole-phrase marker match. Unlike a raw `contains`, this never
/// matches inside another word — e.g. the self-ref marker "my" must NOT match the
/// tail of "autono**my**". Multi-word markers ("am i using") match as an exact
/// word sequence.
fn contains_word_marker(norm: &str, markers: &[&str]) -> bool {
    // Normalize punctuation to spaces (keep apostrophes for "i'm") so "i use?"
    // still matches the marker "i use" while "autonomy" never matches "my".
    let cleaned: String = norm
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect();
    let padded = format!(
        " {} ",
        cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
    );
    markers.iter().any(|m| {
        let mm = m.trim();
        !mm.is_empty() && padded.contains(&format!(" {mm} "))
    })
}

/// True when every word of a field id (`snake_case`) appears as a whole word in
/// the message — i.e. the FIELD itself is referenced, not merely one of its values.
/// Generic (no per-field code): used to guard the verb-less implicit-command path.
fn field_name_referenced(norm: &str, field: &str) -> bool {
    let words: Vec<&str> = norm
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    field
        .split('_')
        .filter(|w| !w.is_empty())
        .all(|fw| words.iter().any(|w| *w == fw))
}

impl SettingsIntentPipeline {
    pub fn new(entity_index: Arc<SchemaEntityIndex>) -> Self {
        Self {
            entity_index,
            thresholds: IntentThresholds::default(),
            deps: crate::config::nl::evidence::EvidenceDeps::default(),
        }
    }
    pub fn with_thresholds(mut self, t: IntentThresholds) -> Self {
        self.thresholds = t;
        self
    }
    /// Inject optional evidence dependencies (embedder / memory / weights). With
    /// none injected the classifier uses the always-available lexical tier and
    /// behaves identically (graceful degradation, golden-preserving).
    pub fn with_evidence(mut self, deps: crate::config::nl::evidence::EvidenceDeps) -> Self {
        self.deps = deps;
        self
    }

    /// Classify a message. `conv` supplies subject/topic signals from history.
    pub fn classify(&self, text: &str, conv: &ConversationContext) -> SettingsDecision {
        self.classify_traced(text, conv).0
    }

    /// Detect a read-only Info intent (catalog/help/explain/recent) from generic
    /// intent phrasing + schema-driven field/group resolution. Returns `None` when
    /// the message is not an info request (so change/read-back logic proceeds).
    fn detect_info(&self, norm: &str, text: &str) -> Option<InfoQuery> {
        // 0) Provider catalog / active-provider read-back (schema-driven).
        let mentions_provider = norm.contains("provider");
        let mentions_model = norm.contains(" model") || norm.starts_with("model");
        let asks_active = norm.contains("am i using")
            || norm.contains("i'm using")
            || norm.contains("current")
            || norm.contains("active")
            || (norm.contains("what") && norm.contains(" my "));
        if mentions_provider || mentions_model {
            let wants_list = norm.contains("which")
                || norm.contains("list")
                || norm.contains("available")
                || norm.contains("what providers")
                || (norm.contains("show") && mentions_provider);
            if mentions_provider && wants_list {
                return Some(InfoQuery::Providers);
            }
            if asks_active {
                return Some(InfoQuery::ActiveProvider);
            }
        }
        // "explain <provider>" → provider catalog. NOTE: only the settings-directed
        // "explain" verb qualifies; a DEFINITIONAL knowledge question that merely
        // names a provider ("what is OpenAI", "tell me about Anthropic") is general
        // knowledge and must fall through to the LLM (Req 6.2, no-interference).
        if norm.starts_with("explain ")
            && crate::llm::provider::config::ProviderType::resolve(norm).is_some()
        {
            return Some(InfoQuery::Providers);
        }

        // 1) Recent changes / history.
        if norm.contains("what changed")
            || norm.contains("recent change")
            || norm.contains("change history")
            || (norm.contains("what did i") && norm.contains("chang"))
            || (norm.contains("show") && norm.contains("recent") && norm.contains("chang"))
        {
            return Some(InfoQuery::RecentChanges { limit: 20 });
        }

        // 2) Help / explain. Two tiers so a KNOWLEDGE question that merely mentions
        // a settings-ish word ("explain how voice recognition works", "what does a
        // GPU do") is NOT mistaken for settings help:
        //   • settings-DIRECTED ("how do I change/set X", "valid values for X",
        //     "why is X locked") → accept a modest field match (domain floor);
        //   • DEFINITIONAL ("explain X", "what does X do") → require a STRONG,
        //     explicit setting reference (a full synonym phrase), else it's a
        //     general knowledge question and must fall through to NotSettings.
        let settings_directed_help = norm.contains("how do i")
            || norm.contains("how to")
            || norm.contains("how can i")
            || norm.contains("where do i")
            || norm.contains("allowed values")
            || norm.contains("valid values")
            || (norm.contains("why is") && norm.contains("lock"));
        let definitional_help = norm.starts_with("explain ") || norm.contains("what does");
        if settings_directed_help || definitional_help {
            if let Some(c) = self.entity_index.best(text) {
                let need = if settings_directed_help {
                    self.thresholds.domain_floor
                } else {
                    // Strong explicit setting reference required for definitional verbs.
                    self.thresholds.act
                };
                if c.score >= need {
                    return Some(InfoQuery::Help {
                        section: c.section,
                        field: c.field,
                    });
                }
            }
        }

        // 3) Catalog / discovery — an enumeration verb AND a settings/section signal.
        let asks = norm.contains("what can i")
            || norm.contains("what can you")
            || norm.contains("what settings")
            || norm.contains("which settings")
            || norm.starts_with("list ")
            || norm.contains("show ")
            || norm.contains("what are my")
            || norm.contains("what options")
            || norm.contains("what are the options")
            || norm.contains("what configuration");
        let settingsish = norm.contains("setting")
            || norm.contains("configur")
            || norm.contains("option")
            || norm.contains("preferen")
            || norm.contains("customize");
        let group = self.resolve_group(norm);
        if asks && (settingsish || group.is_some()) {
            return Some(InfoQuery::Catalog { group });
        }
        None
    }

    /// (helper defined below as a free fn: `field_name_referenced`)

    /// Resolve a config SECTION mentioned in the text (for "list <group> settings"),
    /// driven by the actual section names in the schema — no hardcoded group list.
    fn resolve_group(&self, norm: &str) -> Option<String> {
        let mut seen = std::collections::BTreeSet::new();
        for (section, _field) in crate::config::schema::all_fields() {
            if seen.insert(section.clone()) && norm.contains(&section) {
                return Some(section);
            }
        }
        None
    }

    pub fn classify_traced(
        &self,
        text: &str,
        conv: &ConversationContext,
    ) -> (SettingsDecision, SettingsIntentTrace) {
        let mut trace = SettingsIntentTrace::default();
        let norm = text.to_ascii_lowercase();
        let norm = norm.trim();

        // ── Undo intent (generic verb class; no field needed) ───────────────
        // Guard against user-artifact subject ("revert my code changes").
        let subject = conv.subject_signal(text);
        trace.subject = Some(match subject {
            SubjectSignal::KriaDirected => "kria",
            SubjectSignal::UserArtifact => "user_artifact",
            SubjectSignal::Neutral => "neutral",
        });
        if UNDO_MARKERS.iter().any(|m| norm.contains(m)) && subject != SubjectSignal::UserArtifact {
            let settingsish = norm.contains("setting")
                || norm.contains("config")
                || norm.contains("change")
                || norm.contains("previous")
                || norm.contains("back")
                || norm.contains("undo");
            if settingsish {
                trace.decision = "undo";
                return (SettingsDecision::Undo, trace);
            }
        }

        // ── Info intents (catalog/help/explain/recent) ───────────────────────
        // Generic INTENT verbs (not field keywords) + schema-driven field/group
        // resolution → answer-from-system, never the LLM (Req 5). Guarded so it
        // never fires for a user-artifact subject.
        if subject != SubjectSignal::UserArtifact {
            if let Some(info) = self.detect_info(norm, text) {
                trace.decision = "info";
                return (SettingsDecision::Info(info), trace);
            }
        }

        // ── Content-authoring guard (no-interference, Req 6.2) ───────────────
        // A message that opens with a content-authoring verb ("write code to …",
        // "draw a dark poster") is a generation request, never a settings mutation —
        // even when it names a settings word. Exempt the turn-scoped image-routing
        // phrasing ("generate this using local/cloud"), which legitimately resolves
        // an image_mode value on the value-grounded path.
        let gen_route = norm.contains("generate") && norm.contains("using");
        if !gen_route && CONTENT_LEAD_VERBS.iter().any(|v| norm.starts_with(v)) {
            trace.decision = "not_settings";
            return (SettingsDecision::NotSettings, trace);
        }

        // ── Entity resolution (schema-driven) ────────────────────────────────
        // Pick the candidate whose grounded VALUE resolves (disambiguates e.g.
        // "turn off voice" → voice.enabled over voice.mode).
        let candidates = self.entity_index.resolve(text);
        let mut best: Option<(FieldCandidate, Option<serde_json::Value>, f32)> = None;
        for c in candidates.into_iter().take(6) {
            let value = self.entity_index.resolve_value(&c.section, &c.field, text);
            let mut score = c.score;
            if value.is_some() {
                score = (score + 0.40).min(1.0); // schema-grounded value = strong signal
            }
            if best.as_ref().map(|(_, _, s)| score > *s).unwrap_or(true) {
                best = Some((c, value, score));
            }
        }

        let (cand, value, mut entity_conf) = match best {
            Some(b) => b,
            None => {
                trace.decision = "not_settings";
                return (SettingsDecision::NotSettings, trace);
            }
        };
        trace.best_field = Some((cand.section.clone(), cand.field.clone()));
        trace.entity_score = cand.score;
        trace.value_grounded = value.is_some();

        // ── Configuration-vs-Conversation separation (Req 2) — evidence fusion ─
        // A user-artifact subject is decisive negative evidence.
        if subject == SubjectSignal::UserArtifact {
            trace.decision = "not_settings";
            return (SettingsDecision::NotSettings, trace);
        }
        // Collect NEGATIVE-evidence signals ONLY for a weak, value-less,
        // neutral-subject guess — this is the sole place a settings guess can be
        // suppressed, so the (potentially expensive) embedding/memory calls run
        // ONLY when the decision is genuinely ambiguous. Confident guesses (strong
        // field, grounded value, or a KRIA-directed subject) skip this entirely,
        // keeping ordinary turns fast (performance requirement).
        if subject == SubjectSignal::Neutral && !trace.value_grounded && cand.score < 0.85 {
            let embedder = self.deps.embedder.as_deref();
            let conversation_topic = conv.topic_affinity(text, embedder);
            let memory_topic = self
                .deps
                .memory
                .as_ref()
                .and_then(|m| m.topic_affinity(text))
                .unwrap_or(0.0);
            trace.conversation_topic = conversation_topic;
            trace.memory_topic = memory_topic;
            trace.embeddings_used = embedder.is_some();

            let w = &self.deps.weights;
            let suppress = (w.conversation_penalty * conversation_topic
                + w.memory_penalty * memory_topic)
                .min(0.7);
            if suppress > 0.0 {
                entity_conf *= (1.0 - suppress).max(0.3);
            }
        }
        // Explicit KRIA-directed subject is POSITIVE evidence (default weight 0 →
        // no-op unless tuned).
        if subject == SubjectSignal::KriaDirected {
            entity_conf = (entity_conf + self.deps.weights.subject_kria_boost).min(1.0);
        }

        // ── Intent kind ──────────────────────────────────────────────────────
        let first = norm.split_whitespace().next().unwrap_or("");
        let is_question = norm.ends_with('?')
            || QUESTION_STARTS.iter().any(|q| norm.starts_with(q))
            || norm.contains("am i using")
            || norm.contains("which ");
        // Desire phrasings express a change request without an imperative verb
        // ("I want dark mode", "I'd like a bigger font"). They count as imperative
        // ONLY in combination with a resolved field + value below, so a desire with
        // no settings field ("I want to learn Rust") stays conversation.
        let is_desire = DESIRE_MARKERS
            .iter()
            .any(|m| norm.starts_with(m) || norm.contains(m));
        let is_imperative = IMPERATIVE_VERBS.contains(&first)
            || IMPERATIVE_VERBS
                .iter()
                .any(|v| norm.starts_with(&format!("{v} ")))
            || is_desire;
        // Word-boundary match so "my" doesn't match the tail of "autonomy".
        let self_ref = contains_word_marker(norm, SELF_REF_READBACK);
        // Declarative statement/opinion ("dark mode is ugly") — a comment, not a
        // command. Guards the bare-noun clarify path below.
        let is_statement = STATEMENT_COPULAS.iter().any(|c| norm.contains(c));
        trace.is_question = is_question;
        trace.is_imperative = is_imperative;
        trace.self_ref = self_ref;

        // ── Read-back: a settings QUESTION with a self-reference marker ───────
        // "what is my current theme" (self-ref) ✓ ; "what is dark mode" (no self-ref,
        // definitional) ✗ → falls through to NotSettings.
        if is_question && !is_imperative {
            if self_ref && cand.score >= self.thresholds.domain_floor {
                trace.confidence = (entity_conf + 0.30).min(1.0);
                if trace.confidence >= self.thresholds.act {
                    trace.decision = "read_back";
                    return (
                        SettingsDecision::ReadBack {
                            section: cand.section,
                            field: cand.field,
                        },
                        trace,
                    );
                }
            }
            // Definitional / non-self-ref question about a settings-ish word.
            trace.decision = "not_settings";
            return (SettingsDecision::NotSettings, trace);
        }

        // ── Change / Temp: an imperative settings command ────────────────────
        if is_imperative {
            trace.confidence = entity_conf;
            if trace.confidence >= self.thresholds.act {
                let scope = if TEMP_CUES.iter().any(|c| norm.contains(c))
                    || (norm.contains("generate") && norm.contains("using"))
                {
                    Scope::Temp
                } else {
                    Scope::Permanent
                };
                trace.decision = if scope == Scope::Temp {
                    "temp"
                } else {
                    "change"
                };
                return (
                    SettingsDecision::Change {
                        section: cand.section,
                        field: cand.field,
                        value,
                        scope,
                    },
                    trace,
                );
            }
            if trace.confidence >= self.thresholds.clarify {
                trace.decision = "clarify";
                return (
                    SettingsDecision::Clarify {
                        question: format!(
                            "Did you want to change {}.{}?",
                            cand.section, cand.field
                        ),
                    },
                    trace,
                );
            }
            trace.decision = "not_settings";
            return (SettingsDecision::NotSettings, trace);
        }

        // ── Implicit command: grounded value + strong field, no explicit verb ─
        // Catches non-English imperatives ("theme ko dark karo") and terse commands
        // ("theme: dark"). Guarded by a complaint/negation check so opinions
        // ("I don't like dark mode") stay statements.
        let complaint = COMPLAINT_MARKERS.iter().any(|m| norm.contains(m));
        // Require some command STRUCTURE beyond the bare field phrase: a lone
        // "dark mode" (== the synonym itself) is ambiguous → clarify; "theme ko dark
        // karo" has extra imperative words → command.
        let word_count = norm.split_whitespace().count();
        // The verb-less implicit path must not fire on a mere VALUE word ("create a
        // dark themed poster" mentions "dark" but not the theme field). Require the
        // FIELD to be explicitly referenced (all field-name words present as whole
        // words) — no-interference guard (Req 6.2).
        let field_referenced = field_name_referenced(norm, &cand.field);
        if value.is_some()
            && field_referenced
            && !complaint
            && word_count >= 3
            && entity_conf >= self.thresholds.act
        {
            // Turn-scoped when an explicit temp cue is present OR it's a per-request
            // image-generation routing ("generate this image using local/cloud").
            let scope = if TEMP_CUES.iter().any(|c| norm.contains(c))
                || (norm.contains("generate") && norm.contains("using"))
            {
                Scope::Temp
            } else {
                Scope::Permanent
            };
            trace.confidence = entity_conf;
            trace.decision = if scope == Scope::Temp {
                "temp"
            } else {
                "change"
            };
            return (
                SettingsDecision::Change {
                    section: cand.section,
                    field: cand.field,
                    value,
                    scope,
                },
                trace,
            );
        }

        // ── Bare noun / statement (no verb, not a question) ───────────────────
        // "dark mode" — a SHORT, essentially field-only message with no action ⇒
        // clarify. The same phrase embedded in a longer content sentence
        // ("write a poem about dark mode") is NOT a settings command (Req 6.2).
        trace.confidence = entity_conf;
        if cand.score >= 0.80 && word_count <= 4 && !is_statement {
            trace.decision = "clarify";
            return (
                SettingsDecision::Clarify {
                    question: format!(
                        "Did you want to change or check {}.{}?",
                        cand.section, cand.field
                    ),
                },
                trace,
            );
        }
        trace.decision = "not_settings";
        (SettingsDecision::NotSettings, trace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct GoldenCase {
        prompt: String,
        decision: String,
        section: Option<String>,
        field: Option<String>,
        #[allow(dead_code)]
        value: Option<serde_json::Value>,
        #[allow(dead_code)]
        context: Option<String>,
    }

    fn pipeline() -> SettingsIntentPipeline {
        SettingsIntentPipeline::new(Arc::new(SchemaEntityIndex::build()))
    }

    fn decision_kind(d: &SettingsDecision) -> &'static str {
        match d {
            SettingsDecision::Change { scope, .. } => {
                if *scope == Scope::Temp {
                    "temp"
                } else {
                    "change"
                }
            }
            SettingsDecision::ReadBack { .. } => "read_back",
            SettingsDecision::Info(_) => "info",
            SettingsDecision::Undo => "undo",
            SettingsDecision::Clarify { .. } => "clarify",
            SettingsDecision::NotSettings => "not_settings",
        }
    }

    #[test]
    fn audit_false_positives_are_fixed() {
        let conv = ConversationContext::default();
        let p = pipeline();
        for prompt in [
            "dark mode is ugly",
            "what is OpenAI",
            "write code to change a theme",
            "what is autonomy in philosophy",
        ] {
            let (d, _t) = p.classify_traced(prompt, &conv);
            assert!(
                matches!(d, SettingsDecision::NotSettings),
                "{prompt:?} should be NotSettings, got {d:?}"
            );
        }
        // Guard: legitimate settings phrasings still route.
        assert!(matches!(
            p.classify("explain OpenRouter", &conv),
            SettingsDecision::Info(_)
        ));
        assert!(matches!(
            p.classify("what is my current theme?", &conv),
            SettingsDecision::ReadBack { .. }
        ));
        assert!(matches!(
            p.classify("dark mode", &conv),
            SettingsDecision::Clarify { .. }
        ));
    }

    #[test]
    fn golden_set_classifies_correctly() {
        let raw = include_str!("golden_set.jsonl");
        let conv = ConversationContext::default();
        let mut failures = Vec::new();
        let mut total = 0;
        for line in raw.lines().filter(|l| !l.trim().is_empty()) {
            let case: GoldenCase = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("bad golden line: {line}\n{e}"));
            total += 1;
            let (decision, trace) = pipeline().classify_traced(&case.prompt, &conv);
            let got = decision_kind(&decision);
            // "temp" and "change" both acceptable where the golden marks the other
            // only if scope cue is absent is out of scope; we assert exact kind.
            if got != case.decision {
                failures.push(format!(
                    "  {:?} → expected {}, got {} (conf {:.2}, field {:?})",
                    case.prompt, case.decision, got, trace.confidence, trace.best_field
                ));
                continue;
            }
            // If a field is expected, assert it (change/read_back/temp).
            if let (Some(es), Some(ef)) = (&case.section, &case.field) {
                let field_ok = match &decision {
                    SettingsDecision::Change { section, field, .. }
                    | SettingsDecision::ReadBack { section, field } => section == es && field == ef,
                    _ => true,
                };
                if !field_ok {
                    failures.push(format!(
                        "  {:?} → wrong field: expected {es}.{ef}, got {:?}",
                        case.prompt, trace.best_field
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} / {} golden cases failed:\n{}",
            failures.len(),
            total,
            failures.join("\n")
        );
    }

    #[test]
    fn conversation_context_biases_away_from_settings() {
        // Mid code discussion, a neutral settings-like phrase should not act.
        let conv = ConversationContext::new(
            vec![
                "help me refactor this rust function".into(),
                "fix the compile bug in my code".into(),
            ],
            vec![],
        );
        let d = pipeline().classify("change the theme", &conv);
        // With code topic active + neutral subject + no grounded value, bias down.
        assert!(
            matches!(
                d,
                SettingsDecision::NotSettings | SettingsDecision::Clarify { .. }
            ),
            "got {d:?}"
        );
    }
}
