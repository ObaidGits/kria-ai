//! Live Fact Classifier — 3-Gate Pipeline for Real-Time Fact Detection
//!
//! Determines if a query requires live/current data using three sequential gates:
//!
//! Gate 1: Temporal-Signal Lexical Gate (open-world)
//!   Detects universal temporal signals (current, latest, since, how old, etc.)
//!   that indicate a query needs real-time data regardless of topic.
//!   This is the open-world gate — it catches unseen prompts.
//!
//! Gate 2: Semantic Anchor Match (closed-world, broad coverage)
//!   Uses FastEmbed cosine similarity against anchor embeddings to detect
//!   implicit live-fact queries (e.g., "who is the CM" without temporal signals).
//!
//! Gate 3: Historical Rejection Filter (safety net)
//!   Rejects queries with explicit past-tense markers to prevent false positives.
//!   Runs AFTER Gates 1 & 2 to catch historical queries that slipped through.
//!
//! A query is is_live_fact = true if it passes Gate 1 OR Gate 2, AND Gate 3.

use chrono::Datelike;
use once_cell::sync::Lazy;
use regex::Regex;
use std::env;

use super::embed::{cosine_sim, embed_one};

// ─── Gate 1: Temporal-Signal Lexical Gate ─────────────────────────────────────
// Universal temporal signals that indicate a query needs real-time data.
// These are topic-agnostic — they work for ANY subject (politics, finance, sports, etc.)

static TEMPORAL_SIGNAL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:\bcurrent(?:ly)?\b|\blatest\b|\brecently\b|\bright now\b|\bas of now\b|\bat present\b|\btoday'?s?\b|\btonight\b|\bthis (?:week|month|year|morning|evening)\b|\bnow\b|\blive\b|\breal[- ]?time\b|\bup[- ]?to[- ]?date\b|\bfresh(?:ly)?\b|\bjust now\b|\bhow (?:old|many|long|much)\b|\bsince\b|\bago\b|\byears? (?:since|ago|from now|of|have been)\b|\byears? (?:of|have been)\b.*\bsince\b|\bhave been since\b|\bhow long (?:has|is|since|until|till|been)\b|\bwhat (?:is|are|was|were) the (?:current|latest|present|today'?s?)\b|\bwho is (?:the|currently)\b|\bwho (?:is|are) the (?:current|latest|new|present)\b|\bprice (?:of|for|today)\b|\bscore (?:of|today|now|update)\b|\bresult (?:of|today|now|latest|update)\b|\b(?:stock|bitcoin|crypto|gold|oil|dollar|rupee|euro) (?:price|rate|value)\b|\b(?:population|gdp|inflation|unemployment) (?:of|rate|today|current)\b|\b(?:election|vote|poll|sworn|oath|inauguration|appointed)\b|\b(?:match|game|race|tournament|ipl|nfl|nba|premier) (?:result|score|today|live|update)\b|\b(?:weather|temperature|forecast|rain|storm) (?:today|now|current|update)\b)").expect("Invalid temporal signal regex")
});

/// Gate 1: Detect temporal signals in the query.
/// Returns true if the query contains any universal temporal marker.
/// This is the open-world gate — it catches unseen prompts by detecting
/// the *need for recency* from grammar, not from topic.
fn contains_temporal_signal(query: &str) -> bool {
    TEMPORAL_SIGNAL_RE.is_match(query)
}

/// Self-referential capability/identity check.
///
/// Detects prompts about KRIA's own capabilities ("what can you do",
/// "tell me about yourself", "are you live", "what are your features").
/// These must NEVER trigger live-fact retrieval — KRIA's capabilities are
/// internal knowledge, not external facts.
// PRODUCTION HARDENING FIX (OpenClaw pipeline audit, Phase 3: semantic router
// gap). Root cause, confirmed via real log evidence: "List installed OpenClaw
// skills." scored gate1_temporal=false, gate2_semantic=true — the SEMANTIC
// anchor gate (not this lexical pre-filter) misfired, treating the phrase as
// live-fact-like. This pre-filter was ALREADY the documented, intended
// mechanism to prevent exactly this class of self-referential-capability
// query from reaching the live-fact gates at all — but it only recognized
// "you"/"your"/"yourself"/"kria" as self-referential subjects, never
// "openclaw"/"skill(s)"/"tool(s)", so a query entirely about KRIA's own
// OpenClaw skill registry (which IS self-referential capability knowledge,
// not an external live fact) fell through to Gate 2 and got hijacked into a
// forced `#tool:searxng_search` directive — the exact mechanism behind the
// "List the skills available in the OpenClaw marketplace" →
// `mcp_fs_list_directory`/`web_search` misrouting confirmed in real GUI usage.
static SELF_REF_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:you|your|yourself|kria|marketplace)\b").expect("Invalid self-ref regex")
});

static CAPABILITY_VERB_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:can|could|able|capable|capabilit(?:y|ies)|abilit(?:y|ies)|feature|features|do|help|assist|support|name|identity|model|live|alive|online|ready|awake|version|who|what|skill|skills|list|installed|enabled|disabled|marketplace|generated)\b")
        .expect("Invalid capability verb regex")
});

/// Subject terms that are UNCONDITIONALLY self-referential to KRIA's own
/// system regardless of the rest of the sentence — "openclaw"/"clawhub" name
/// KRIA's own skill subsystem specifically (unlike "you"/"kria" which are
/// broad enough that requiring a capability-verb co-occurrence avoids
/// over-matching ordinary sentences that happen to say "kria" or "you").
///
/// PRODUCTION HARDENING FIX (OpenClaw pipeline audit, Phase 3). Real
/// production failure, confirmed via live backend logs: "Use OpenClaw to
/// evaluate the expression 8 * 8" matched `SELF_REF_RE` (contains "openclaw")
/// but NOT `CAPABILITY_VERB_RE` (no can/do/help/skill/etc. word), so the
/// original `&&`-gated check failed and the query still got hijacked into a
/// forced `#tool:searxng_search` directive. Any mention of OpenClaw/ClawHub is
/// unconditionally about KRIA's own subsystem — it can never be an external
/// live fact — so it must not additionally require a capability verb.
static UNCONDITIONAL_SELF_REF_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:openclaw|clawhub)\b").expect("Invalid unconditional self-ref regex")
});

fn is_self_referential_capability_query(query: &str) -> bool {
    UNCONDITIONAL_SELF_REF_RE.is_match(query)
        || (SELF_REF_RE.is_match(query) && CAPABILITY_VERB_RE.is_match(query))
}

/// Bare OpenClaw skill-management phrasing that never mentions
/// "you"/"kria"/"openclaw" explicitly (e.g. "which skills are enabled") but
/// is still unambiguously about KRIA's own skill registry, not an external
/// live fact. Requires a skill-management action word AND "skill"/"skills"
/// so generic unrelated sentences containing "skill" alone don't match.
static SKILL_MANAGEMENT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:list|show|which|what)\b.{0,30}\bskills?\b.{0,30}\b(?:installed|enabled|disabled|available|active|inactive)\b|\b(?:installed|enabled|disabled|available)\b.{0,30}\bskills?\b")
        .expect("Invalid skill management regex")
});

fn is_skill_management_query(query: &str) -> bool {
    SKILL_MANAGEMENT_RE.is_match(query)
}

/// Skill-invocation phrasing ("use/run/execute the skill called X", "run
/// skill X") — real production failure, confirmed via live backend logs:
/// "Use the skill called oc_this_skill_does_not_exist_99999 to do something"
/// matched none of the other pre-filters (no "openclaw"/"kria" mention, no
/// installed/enabled/disabled keyword) and got hijacked into
/// `#tool:searxng_search`. Any phrase invoking "the skill (called/named) X"
/// is unambiguously an OpenClaw skill-invocation request, never an external
/// live fact.
static SKILL_INVOCATION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:use|run|execute|invoke)\b.{0,15}\bthe\s+skill\b|\bskill\s+(?:called|named)\b",
    )
    .expect("Invalid skill invocation regex")
});

fn is_skill_invocation_query(query: &str) -> bool {
    SKILL_INVOCATION_RE.is_match(query)
}

// ─── Gate 2: Semantic Anchor Embeddings ───────────────────────────────────────
// Expanded anchor phrases covering the full space of volatile queries.
// Pre-embedded at first call via OnceLock — zero per-turn overhead after warmup.

const LIVE_FACT_ANCHORS_TEXT: &[&str] = &[
    // Political leadership
    "who is the current chief minister",
    "who is the current prime minister",
    "who is the president of",
    "current political leader",
    "who won the election",
    "latest election result",
    "who took oath today",
    "breaking news about government",
    // Financial markets
    "current stock price",
    "bitcoin price today",
    "exchange rate now",
    "current market value",
    // Sports
    "live sports score",
    "match result today",
    "ipl score today",
    // Temporal-drift calculations
    "how many years since",
    "how old is",
    "current age of",
    "years since independence",
    // Statistics that change
    "population of",
    "current GDP",
    "unemployment rate",
    // Current events
    "latest update on",
    "what happened today",
    "current news about",
    // Time-relative
    "what is today's date",
    "how long until",
    "current time",
];

static ANCHOR_EMBEDDINGS: Lazy<Option<Vec<Vec<f32>>>> =
    Lazy::new(|| match embed_batch(LIVE_FACT_ANCHORS_TEXT) {
        Ok(vectors) => Some(vectors),
        Err(_) => {
            tracing::warn!(
                "[LiveFactClassifier] Failed to pre-embed anchors; semantic layer disabled"
            );
            None
        }
    });

fn embed_batch(texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
    super::embed::embed_batch(texts).map_err(|e| e.to_string())
}

fn anchors() -> Vec<Vec<f32>> {
    ANCHOR_EMBEDDINGS.as_ref().cloned().unwrap_or_default()
}

// ─── Gate 3: Historical Rejection Filter ──────────────────────────────────────
// Rejects queries with explicit past-tense markers to prevent false positives.

static HISTORICAL_TENSE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(who was|who were|what was|what were|history of|historical|former|previous|past|back in|during the)\b")
        .expect("Invalid historical tense regex")
});

static PAST_YEAR_RE: Lazy<Regex> = Lazy::new(|| {
    let current_year = chrono::Utc::now().year();
    // Build a pattern that matches years from 1900 to current_year - 1
    // For 2026: matches 1900-2025
    let year_pattern = if current_year > 2000 {
        let last_two = current_year - 2000;
        if last_two <= 9 {
            format!(r"(?:19\d{{2}}|200[0-{}])", last_two - 1)
        } else {
            let tens = last_two / 10;
            let ones = last_two % 10;
            let mut parts = vec!["19\\d{2}".to_string()];
            for decade in 0..tens {
                if decade == 0 {
                    parts.push("200\\d".to_string());
                } else {
                    parts.push(format!("20{}\\d", decade));
                }
            }
            // Current decade: match up to (ones - 1)
            if ones > 0 {
                parts.push(format!("20{}[0-{}]", tens, ones - 1));
            }
            format!("(?:{})", parts.join("|"))
        }
    } else {
        r"(?:19\d{2})".to_string()
    };
    Regex::new(&format!(r"\b{}\b", year_pattern)).expect("Invalid past year regex")
});

/// Gate 3: Returns true if query contains historical markers (past-tense, old years).
fn contains_historical_markers(query: &str) -> bool {
    let query_lower = query.to_lowercase();
    HISTORICAL_TENSE_RE.is_match(&query_lower) || PAST_YEAR_RE.is_match(&query_lower)
}

// ─── Threshold Configuration ─────────────────────────────────────────────────

const DEFAULT_LIVE_FACT_THRESHOLD: f32 = 0.72;

/// Minimum query length (chars) for Gate 2 semantic matching.
/// Short inputs (greetings, single words) are never live-fact queries.
const MIN_QUERY_LENGTH_FOR_GATE2: usize = 8;

fn get_threshold() -> f32 {
    env::var("KRIA_LIVE_FACT_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(DEFAULT_LIVE_FACT_THRESHOLD)
}

// ─── Public API ─────────────────────────────────────────────────────────────────

/// Determine if a query requires live-fact resolution.
///
/// 3-Gate Pipeline:
/// 1. Gate 1 (Temporal Signal): Open-world lexical gate catches any query with
///    temporal markers (current, latest, since, how old, etc.) regardless of topic.
///    If Gate 1 matches → proceed to Gate 3.
/// 2. Gate 2 (Semantic Anchor): Closed-world semantic match against anchor embeddings.
///    If cosine similarity >= threshold → proceed to Gate 3.
/// 3. Gate 3 (Historical Rejection): Rejects queries with past-tense markers.
///    If Gate 3 matches → return false (historical query, not live fact).
///
/// Returns true if (Gate 1 OR Gate 2) passes AND Gate 3 does not reject.
pub fn is_live_fact_query(query: &str) -> bool {
    // ── Pre-filter: self-referential capability/identity prompts ──
    // Questions about KRIA's own abilities, features, or identity must
    // NEVER trigger live-fact retrieval. They are answered from internal
    // knowledge, not external search.
    if is_self_referential_capability_query(query) {
        tracing::info!(
            query = %query,
            "LiveFactClassifier: self-referential capability prompt — not a live fact"
        );
        return false;
    }

    // ── Pre-filter: GUI / app-launch queries ──────────────────────────────
    // "Open Chrome and search for X", "launch Firefox", "open YouTube" etc.
    // are GUI automation requests, never live-fact queries. The live-fact
    // classifier must not hijack these and force searxng_search.
    // Uses the GuiIntentClassifier (structural signal scoring, not keyword lists).
    if crate::routing::gui_intent::is_gui_launch_query(query) {
        tracing::info!(
            query = %query,
            "LiveFactClassifier: GUI/app-launch query — not a live fact"
        );
        return false;
    }

    // ── Pre-filter: OpenClaw skill-management queries ──────────────────────
    // PRODUCTION HARDENING FIX (OpenClaw pipeline audit, Phase 3). Real
    // production failure, confirmed via live backend logs: "Show me which
    // skills are currently enabled" scored gate2_semantic=true and got
    // hijacked into a forced #tool:searxng_search directive. Bare skill-
    // management phrasing ("list/show/which skills ... installed/enabled/
    // disabled/available") never mentions "you"/"kria"/"openclaw" explicitly,
    // so it can't be caught by the self-referential-subject regex above —
    // but it is unambiguously a query about KRIA's own OpenClaw registry, not
    // an external live fact, regardless of phrasing.
    if is_skill_management_query(query) {
        tracing::info!(
            query = %query,
            "LiveFactClassifier: OpenClaw skill-management query — not a live fact"
        );
        return false;
    }

    if is_skill_invocation_query(query) {
        tracing::info!(
            query = %query,
            "LiveFactClassifier: OpenClaw skill-invocation query — not a live fact"
        );
        return false;
    }

    // ── Pre-filter: Code generation / execution prompts ───────────────────
    // "Create a Python script...", "Write a program...", "Generate code..."
    // are code-generation requests, never live-fact queries.
    {
        let lower = query.to_lowercase();
        let code_gen_signals = [
            "create a python",
            "create a rust",
            "create a javascript",
            "write a python",
            "write a rust",
            "write a program",
            "write a script",
            "write a function",
            "write code",
            "generate a python",
            "generate a script",
            "generate code",
            "run it",
            "run the",
            "execute it",
            "compile it",
            "fibonacci",
            "factorial",
            "hello world",
            "calculator",
            "open code",
            "open vs code",
            "open vscode",
        ];
        if code_gen_signals.iter().any(|s| lower.contains(s)) {
            tracing::info!(
                query = %query,
                "LiveFactClassifier: code generation/execution query — not a live fact"
            );
            return false;
        }
    }

    // Gate 1: Temporal-signal lexical gate (open-world, catches unseen prompts)
    let gate1_temporal = contains_temporal_signal(query);

    // Gate 2: Semantic anchor match (closed-world, broad coverage)
    // Only run if Gate 1 didn't trigger (optimization: skip embedding if already matched)
    let gate2_semantic = if gate1_temporal {
        false // No need to run Gate 2 if Gate 1 already matched
    } else {
        semantic_anchor_match(query)
    };

    // If neither gate triggered, not a live-fact query
    if !gate1_temporal && !gate2_semantic {
        tracing::info!(
            query = %query,
            gate1_temporal = gate1_temporal,
            gate2_semantic = gate2_semantic,
            "LiveFactClassifier: no gate triggered, not a live-fact query"
        );
        return false;
    }

    // Gate 3: Historical rejection (safety net)
    if contains_historical_markers(query) {
        tracing::info!(
            query = %query,
            gate1_temporal = gate1_temporal,
            gate2_semantic = gate2_semantic,
            "LiveFactClassifier: rejected by Gate 3 (historical markers)"
        );
        return false;
    }

    tracing::info!(
        query = %query,
        gate1_temporal = gate1_temporal,
        gate2_semantic = gate2_semantic,
        "LiveFactClassifier: is_live_fact = true"
    );

    true
}

/// Gate 2: Semantic anchor match using FastEmbed cosine similarity.
/// Returns true if the query's embedding is close enough to any anchor embedding.
fn semantic_anchor_match(query: &str) -> bool {
    // Short inputs (greetings, single words, typos) are never live-fact queries.
    // "Hye", "Hi", "Hello", "ok" etc. must not trigger semantic matching.
    if query.trim().chars().count() < MIN_QUERY_LENGTH_FOR_GATE2 {
        tracing::debug!(
            query = %query,
            min_length = MIN_QUERY_LENGTH_FOR_GATE2,
            "LiveFactClassifier: Gate 2 skipped — query too short"
        );
        return false;
    }

    let query_vec = match embed_one(query) {
        Ok(v) => v,
        Err(e) => {
            tracing::info!(
                query = %query,
                error = %e,
                "LiveFactClassifier: Gate 2 embedding failed"
            );
            return false;
        }
    };

    let threshold = get_threshold();
    let anchors_vec = anchors();

    if anchors_vec.is_empty() {
        tracing::info!("LiveFactClassifier: no anchor embeddings available for Gate 2");
        return false;
    }

    let max_sim = anchors_vec
        .iter()
        .map(|anchor| cosine_sim(&query_vec, anchor))
        .fold(0.0f32, f32::max);

    let result = max_sim >= threshold;
    tracing::info!(
        query = %query,
        max_similarity = max_sim,
        threshold = threshold,
        gate2_result = result,
        "LiveFactClassifier: Gate 2 semantic anchor match"
    );

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Gate 1: Temporal Signal Tests ──────────────────────────────────────

    #[test]
    fn test_gate1_temporal_signals() {
        assert!(contains_temporal_signal(
            "How many years since Indian independence?"
        ));
        assert!(contains_temporal_signal(
            "Who is the current chief minister?"
        ));
        assert!(contains_temporal_signal("What is the latest news?"));
        assert!(contains_temporal_signal("Bitcoin price today"));
        assert!(contains_temporal_signal("Live cricket score"));
        assert!(contains_temporal_signal("How old is the president?"));
        assert!(contains_temporal_signal("Population of India currently"));
        assert!(contains_temporal_signal("What happened right now?"));
        assert!(contains_temporal_signal("Stock price of Apple"));
        assert!(contains_temporal_signal("Election result update"));
        assert!(contains_temporal_signal("Weather today"));
        assert!(contains_temporal_signal("GDP of India current"));
        assert!(contains_temporal_signal("How long has it been since 1947?"));
        assert!(contains_temporal_signal("Who is the current PM?"));
        // Awkward phrasings from real user queries
        assert!(contains_temporal_signal(
            "how many years of Indian Independence have been since?"
        ));
        assert!(contains_temporal_signal(
            "How many Yeas of Independence have been since today?"
        ));
        assert!(contains_temporal_signal(
            "years of independence have been since"
        ));
    }

    #[test]
    fn test_gate1_no_temporal_signal() {
        assert!(!contains_temporal_signal("What is photosynthesis?"));
        assert!(!contains_temporal_signal("Explain quantum mechanics"));
        assert!(!contains_temporal_signal("Write a poem about rain"));
    }

    // ─── Gate 3: Historical Rejection Tests ──────────────────────────────────

    #[test]
    fn test_gate3_historical_rejection() {
        assert!(contains_historical_markers(
            "Who was the chief minister in 1995?"
        ));
        assert!(contains_historical_markers(
            "History of Tamil Nadu politics"
        ));
        assert!(contains_historical_markers("Former president of India"));
        assert!(!contains_historical_markers(
            "Who is the chief minister now?"
        ));
    }

    #[test]
    fn test_gate3_past_years() {
        let current_year = chrono::Utc::now().year();
        assert!(contains_historical_markers("Chief minister in 2003"));
        assert!(contains_historical_markers("President in 1999"));
        assert!(!contains_historical_markers(&format!(
            "Chief minister in {}",
            current_year
        )));
    }

    // ─── Integration: 3-Gate Pipeline Tests ─────────────────────────────────

    #[test]
    fn test_live_fact_temporal_drift_queries() {
        // These should be caught by Gate 1 (temporal signals)
        assert!(is_live_fact_query(
            "How many years since Indian independence?"
        ));
        assert!(is_live_fact_query("How old is Narendra Modi?"));
        assert!(is_live_fact_query(
            "What is the current population of India?"
        ));
        assert!(is_live_fact_query("Bitcoin price today"));
        assert!(is_live_fact_query("Live cricket score"));
        assert!(is_live_fact_query("Latest news on elections"));
        assert!(is_live_fact_query("Weather forecast today"));
    }

    #[test]
    fn test_live_fact_historical_rejected() {
        // These should be rejected by Gate 3 even if Gate 1/2 triggers
        assert!(!is_live_fact_query("Who was the chief minister in 1995?"));
        assert!(!is_live_fact_query("History of the prime minister"));
        assert!(!is_live_fact_query(
            "How many years since 1995 independence?"
        ));
    }

    #[test]
    fn test_live_fact_non_temporal_rejected() {
        // These should not trigger any gate
        assert!(!is_live_fact_query("What is photosynthesis?"));
        assert!(!is_live_fact_query("Explain quantum mechanics"));
    }

    #[test]
    fn test_live_fact_short_inputs_rejected() {
        // Short greetings and single words must never be classified as live-fact
        assert!(!is_live_fact_query("Hye"));
        assert!(!is_live_fact_query("Hi"));
        assert!(!is_live_fact_query("Hello"));
        assert!(!is_live_fact_query("ok"));
        assert!(!is_live_fact_query("hey"));
        assert!(!is_live_fact_query("yo"));
        assert!(!is_live_fact_query("test"));
    }

    // ── Self-referential capability prompts: NEVER live-fact ──

    #[test]
    fn test_live_fact_self_referential_capability_rejected() {
        // Self-referential capability/identity prompts must NEVER trigger live-fact,
        // even if they contain temporal trigger words like "currently", "now", "today".
        assert!(!is_live_fact_query("what can you do"));
        assert!(!is_live_fact_query("what can you currently do"));
        assert!(!is_live_fact_query("what are your latest features"));
        assert!(!is_live_fact_query("are you live now"));
        assert!(!is_live_fact_query("what is your current model"));
        assert!(!is_live_fact_query("tell me about yourself"));
        assert!(!is_live_fact_query("who are you"));
        assert!(!is_live_fact_query("what's your current version"));
        assert!(!is_live_fact_query("what is kria"));
    }

    #[test]
    fn test_live_fact_real_temporal_questions_still_pass() {
        // External temporal questions WITHOUT self-reference must still trigger
        assert!(is_live_fact_query("what is the current bitcoin price"));
        assert!(is_live_fact_query("latest news about elections"));
        assert!(is_live_fact_query("how old is the president"));
    }

    /// PRODUCTION HARDENING regression (OpenClaw pipeline audit, Phase 3).
    /// Real production failure, confirmed via live backend logs: "List
    /// installed OpenClaw skills." scored `gate1_temporal=false,
    /// gate2_semantic=true` and got hijacked into a forced
    /// `#tool:searxng_search` directive BEFORE ever reaching the real
    /// OpenClaw semantic router — a self-referential query about KRIA's own
    /// skill registry must never be treated as an external live fact.
    #[test]
    fn regr_openclaw_skill_discovery_queries_never_treated_as_live_fact() {
        assert!(!is_live_fact_query("List installed OpenClaw skills."));
        assert!(!is_live_fact_query("What OpenClaw skills are installed?"));
        assert!(!is_live_fact_query(
            "List the skills available in the OpenClaw marketplace"
        ));
        assert!(!is_live_fact_query(
            "Show me which skills are currently enabled"
        ));
        assert!(!is_live_fact_query(
            "Search the marketplace for a code sandbox skill"
        ));
        assert!(!is_live_fact_query(
            "What generated skills are currently installed?"
        ));
        assert!(!is_live_fact_query(
            "Is there a word-count skill installed?"
        ));
    }

    /// PRODUCTION HARDENING regression (OpenClaw pipeline audit, Phase 3).
    /// Real production failure #2, confirmed via live backend logs after the
    /// first fix landed: "Use OpenClaw to evaluate the expression 8 * 8"
    /// contains "openclaw" but has no capability-verb word (no can/do/help/
    /// skill/etc.), so the original `&&`-gated self-referential check missed
    /// it and it still got hijacked into a forced `#tool:searxng_search`
    /// directive. ANY mention of OpenClaw/ClawHub must be unconditionally
    /// self-referential — it can never be an external live fact regardless
    /// of what else the sentence says.
    #[test]
    fn regr_openclaw_mentions_are_unconditionally_self_referential() {
        assert!(!is_live_fact_query(
            "Use OpenClaw to evaluate the expression 8 * 8"
        ));
        assert!(!is_live_fact_query(
            "Use the openclaw calculator skill on 3+3"
        ));
        assert!(!is_live_fact_query(
            "Route this to OpenClaw: reverse the word kria"
        ));
        assert!(!is_live_fact_query("openclaw 8 * 8"));
        assert!(!is_live_fact_query("Ask ClawHub for a code sandbox"));
    }

    /// PRODUCTION HARDENING regression (OpenClaw pipeline audit, Phase 3).
    /// Real production failure #3: bare "use/run the skill called X" phrasing
    /// with no "openclaw"/"kria" mention and no installed/enabled/disabled
    /// keyword was not caught by any pre-filter and got hijacked into a
    /// forced `#tool:searxng_search` directive.
    #[test]
    fn regr_skill_invocation_phrasing_never_treated_as_live_fact() {
        assert!(!is_live_fact_query(
            "Use the skill called oc_this_skill_does_not_exist_99999 to do something"
        ));
        assert!(!is_live_fact_query("Run the skill named calculator"));
        assert!(!is_live_fact_query("Execute the skill called web_search"));
        assert!(!is_live_fact_query(
            "Uninstall a skill called oc_nonexistent_test_skill_xyz"
        ));
    }

    // ── GUI / app-launch queries: NEVER live-fact ──

    #[test]
    fn test_live_fact_gui_launch_queries_rejected() {
        // These are GUI automation requests — must never trigger live-fact rewrite
        assert!(!is_live_fact_query("Open chrome and search for youtube"));
        assert!(!is_live_fact_query(
            "open chrome and search for youtube live"
        ));
        assert!(!is_live_fact_query("launch Firefox and go to google.com"));
        assert!(!is_live_fact_query("open YouTube"));
        assert!(!is_live_fact_query("start chrome"));
        assert!(!is_live_fact_query(
            "open chrome and search for today's news"
        ));
        assert!(!is_live_fact_query("launch browser and find latest scores"));
        // Text editor / IDE launches with action verbs
        assert!(!is_live_fact_query(
            "Open gedit and type a program to print fibonacci series in python"
        ));
        assert!(!is_live_fact_query(
            "Open code and type a program to print fibonacci series and run it"
        ));
        assert!(!is_live_fact_query(
            "launch vscode and open the project folder"
        ));
    }
}
