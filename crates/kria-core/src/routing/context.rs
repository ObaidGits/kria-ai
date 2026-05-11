//! Context-aware routing.
//!
//! Maintains conversation context between routing decisions to enable:
//! - Topic continuation (carry domain across turns)
//! - Correction detection ("no, I meant X")
//! - Ambiguity resolution via context signals
//!
//! This module is pure string manipulation + lightweight state — no models,
//! no embeddings, no latency impact.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

use super::domain::Domain;
use super::verbs::IntentModality;

// ─── Constants ──────────────────────────────────────────────────────────────

/// Maximum age of context before it's considered stale.
const CONTEXT_STALE_DURATION: Duration = Duration::from_secs(60);

/// Maximum input length for context enrichment (longer inputs are self-sufficient).
const MAX_ENRICH_LENGTH: usize = 80;

/// Minimum input length for context enrichment (very short inputs are too ambiguous).
const MIN_ENRICH_LENGTH: usize = 3;

/// Minimum topic turns before context carries (need at least 1 prior turn).
const MIN_TOPIC_TURNS: usize = 1;

// ─── Correction Detection ───────────────────────────────────────────────────

/// Correction patterns (multilingual: English + Hindi/Hinglish).
static CORRECTION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(no|nahi|nahin|nah|wrong|galat|actually|I meant|I mean|mera matlab|the other|wo nahi|ye nahi|doosra|change to|switch to|update to|not that|not this)\b",
    )
    .expect("valid correction regex")
});

/// Signal detected from user input about correction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CorrectionSignal {
    /// No correction detected.
    None,
    /// Explicit correction phrase found.
    Explicit {
        /// The raw correction text.
        text: String,
    },
}

impl CorrectionSignal {
    pub fn is_correction(&self) -> bool {
        matches!(self, Self::Explicit { .. })
    }
}

/// Detect if user is correcting the previous routing decision.
///
/// # Examples
///
/// ```
/// use kria_core::routing::context::{detect_correction, CorrectionSignal, RoutingContext};
///
/// let ctx = RoutingContext::default();
/// let signal = detect_correction("no I meant the network", &ctx);
/// assert!(signal.is_correction());
///
/// let signal = detect_correction("open Chrome", &ctx);
/// assert_eq!(signal, CorrectionSignal::None);
/// ```
pub fn detect_correction(text: &str, ctx: &RoutingContext) -> CorrectionSignal {
    // Only detect corrections if there was a previous routing decision
    if ctx.last_domain.is_none() {
        return CorrectionSignal::None;
    }

    if CORRECTION_RE.is_match(text) {
        CorrectionSignal::Explicit {
            text: text.to_string(),
        }
    } else {
        CorrectionSignal::None
    }
}

// ─── Context Enrichment ─────────────────────────────────────────────────────

/// Enrichment reason for debugging / tracing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnrichmentReason {
    /// No enrichment applied.
    None,
    /// Correction signal prepended.
    Correction,
    /// Topic continuation (domain carried from previous turn).
    TopicContinuation,
}

/// Result of context enrichment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedInput {
    /// The (possibly enriched) text to route.
    pub text: String,
    /// Original text before enrichment.
    pub original: String,
    /// Why enrichment was applied.
    pub reason: EnrichmentReason,
}

impl EnrichedInput {
    /// Create an enriched input with context injected.
    pub fn enriched(text: String, original: String, reason: EnrichmentReason) -> Self {
        Self {
            text,
            original,
            reason,
        }
    }

    /// Create an input with no enrichment.
    pub fn original(text: &str) -> Self {
        Self {
            text: text.to_string(),
            original: text.to_string(),
            reason: EnrichmentReason::None,
        }
    }

    /// The effective text to route (enriched or original).
    pub fn effective_text(&self) -> &str {
        &self.text
    }
}

/// Enrich user text with routing context for better embedding quality.
///
/// This function is the core of context-aware routing. It prepends context
/// signals to short/ambiguous inputs so the embedding model can resolve them
/// correctly using the conversation history.
///
/// # Enrichment Rules
///
/// 1. Stale context (>60s) → no enrichment
/// 2. Long input (>80 chars) → no enrichment (already specific enough)
/// 3. Correction detected → prepend correction signal
/// 4. Short ambiguous input + strong context → carry domain anchor
///
/// # Examples
///
/// ```
/// use kria_core::routing::context::*;
/// use kria_core::routing::domain::Domain;
/// use kria_core::routing::verbs::IntentModality;
///
/// let mut ctx = RoutingContext::default();
/// ctx.record_turn(Domain::SystemInfo, Some("check_system_health".into()), IntentModality::Read, vec![0.1; 384]);
///
/// let enriched = enrich_with_context("also check disk", &ctx);
/// assert!(enriched.text.contains("system")); // context injected
/// ```
pub fn enrich_with_context(text: &str, ctx: &RoutingContext) -> EnrichedInput {
    // Rule 0: Don't enrich empty or very short inputs
    if text.trim().len() < MIN_ENRICH_LENGTH {
        return EnrichedInput::original(text);
    }

    // Rule 1: Don't enrich if context is stale
    if ctx.is_stale() {
        return EnrichedInput::original(text);
    }

    // Rule 2: Don't enrich long inputs (they're self-sufficient)
    if text.len() > MAX_ENRICH_LENGTH {
        return EnrichedInput::original(text);
    }

    // Rule 3: Correction — prepend correction signal
    if ctx.correction_pending {
        let enriched_text = format!("[correction] {}", text);
        return EnrichedInput::enriched(
            enriched_text,
            text.to_string(),
            EnrichmentReason::Correction,
        );
    }

    // Rule 4: Topic continuation — carry domain anchor for short inputs
    if ctx.last_domain.is_some() && ctx.turn_count_in_topic >= MIN_TOPIC_TURNS && text.len() < 40 {
        if let Some(domain) = ctx.last_domain {
            let domain_hint = domain.anchor_sentences()[0];
            let enriched_text = format!("{} [context: {}]", text, domain_hint);
            return EnrichedInput::enriched(
                enriched_text,
                text.to_string(),
                EnrichmentReason::TopicContinuation,
            );
        }
    }

    // No enrichment needed
    EnrichedInput::original(text)
}

// ─── Routing Context ────────────────────────────────────────────────────────

/// Conversation context carried between routing decisions.
///
/// This struct tracks the state of the current conversation to enable
/// context-aware routing. It's maintained by `TurnGate` and passed to
/// the router on each turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingContext {
    /// Domain of the previous turn (if successfully routed).
    pub last_domain: Option<Domain>,
    /// Tool name of the previous turn (if directly matched).
    pub last_tool: Option<String>,
    /// Modality of the previous turn.
    pub last_modality: IntentModality,
    /// How many consecutive turns in the same domain.
    pub turn_count_in_topic: usize,
    /// Whether the user explicitly corrected the previous routing.
    pub correction_pending: bool,
    /// Timestamp of the last turn (for staleness detection).
    /// Note: Serialized as unix millis, deserialized back.
    #[serde(skip)]
    pub last_turn_at: Option<Instant>,
    /// The embedding of the previous turn (for similarity carry).
    #[serde(skip)]
    pub last_embedding: Option<Vec<f32>>,
}

impl Default for RoutingContext {
    fn default() -> Self {
        Self {
            last_domain: None,
            last_tool: None,
            last_modality: IntentModality::Unknown,
            turn_count_in_topic: 0,
            correction_pending: false,
            last_turn_at: None,
            last_embedding: None,
        }
    }
}

impl RoutingContext {
    /// Update context after a successful routing decision.
    ///
    /// If the new domain matches the previous domain, increment the topic
    /// counter. Otherwise, reset it to 1 (new topic started).
    pub fn record_turn(
        &mut self,
        domain: Domain,
        tool: Option<String>,
        modality: IntentModality,
        embedding: Vec<f32>,
    ) {
        // Check if topic changed
        if let Some(last) = self.last_domain {
            if last == domain {
                self.turn_count_in_topic += 1;
            } else {
                // New topic — reset counter
                self.turn_count_in_topic = 1;
            }
        } else {
            // First turn — set counter to 1
            self.turn_count_in_topic = 1;
        }

        self.last_domain = Some(domain);
        self.last_tool = tool;
        self.last_modality = modality;
        self.last_embedding = Some(embedding);
        self.last_turn_at = Some(Instant::now());
        self.correction_pending = false; // Clear correction flag after recording
    }

    /// Mark that the user is correcting the previous routing.
    pub fn set_correction_pending(&mut self) {
        self.correction_pending = true;
    }

    /// Reset context on explicit topic change or long silence.
    pub fn reset(&mut self) {
        self.last_domain = None;
        self.last_tool = None;
        self.last_modality = IntentModality::Unknown;
        self.turn_count_in_topic = 0;
        self.correction_pending = false;
        self.last_turn_at = None;
        self.last_embedding = None;
    }

    /// Check if context is stale (>60s since last turn).
    pub fn is_stale(&self) -> bool {
        match self.last_turn_at {
            Some(at) => at.elapsed() > CONTEXT_STALE_DURATION,
            None => true, // No previous turn → stale
        }
    }

    /// Check if context has a strong previous domain (for topic continuation).
    pub fn has_strong_context(&self) -> bool {
        self.last_domain.is_some()
            && self.turn_count_in_topic >= MIN_TOPIC_TURNS
            && !self.is_stale()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_context_is_empty() {
        let ctx = RoutingContext::default();
        assert!(ctx.last_domain.is_none());
        assert_eq!(ctx.turn_count_in_topic, 0);
        assert!(!ctx.correction_pending);
    }

    #[test]
    fn record_turn_sets_domain() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            Some("check_system_health".into()),
            IntentModality::Read,
            vec![0.1; 10],
        );
        assert_eq!(ctx.last_domain, Some(Domain::SystemInfo));
        assert_eq!(ctx.turn_count_in_topic, 1);
    }

    #[test]
    fn record_turn_increments_topic_count() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );
        assert_eq!(ctx.turn_count_in_topic, 3);
    }

    #[test]
    fn record_turn_resets_on_topic_change() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );
        assert_eq!(ctx.turn_count_in_topic, 3);

        ctx.record_turn(Domain::Comms, None, IntentModality::Send, vec![0.2; 10]);
        assert_eq!(ctx.turn_count_in_topic, 1); // reset to 1
        assert_eq!(ctx.last_domain, Some(Domain::Comms));
    }

    #[test]
    fn context_is_stale_without_previous_turn() {
        let ctx = RoutingContext::default();
        assert!(ctx.is_stale());
    }

    #[test]
    fn context_is_not_stale_immediately() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );
        assert!(!ctx.is_stale());
    }

    #[test]
    fn reset_clears_everything() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            Some("tool".into()),
            IntentModality::Read,
            vec![0.1; 10],
        );
        ctx.set_correction_pending();
        ctx.reset();

        assert!(ctx.last_domain.is_none());
        assert!(ctx.last_tool.is_none());
        assert_eq!(ctx.turn_count_in_topic, 0);
        assert!(!ctx.correction_pending);
    }

    #[test]
    fn correction_detection_with_context() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );

        let signal = detect_correction("no I meant the network", &ctx);
        assert!(signal.is_correction());
    }

    #[test]
    fn correction_detection_without_context() {
        let ctx = RoutingContext::default();
        let signal = detect_correction("no I meant the network", &ctx);
        assert_eq!(signal, CorrectionSignal::None);
    }

    #[test]
    fn no_correction_on_normal_input() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );

        let signal = detect_correction("check the disk", &ctx);
        assert_eq!(signal, CorrectionSignal::None);
    }

    #[test]
    fn enrichment_topic_continuation() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );

        let enriched = enrich_with_context("also check disk", &ctx);
        assert!(enriched.text.contains("system")); // domain anchor injected
        assert_eq!(enriched.reason, EnrichmentReason::TopicContinuation);
    }

    #[test]
    fn enrichment_correction() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );
        ctx.set_correction_pending();

        let enriched = enrich_with_context("the network", &ctx);
        assert!(enriched.text.starts_with("[correction]"));
        assert_eq!(enriched.reason, EnrichmentReason::Correction);
    }

    #[test]
    fn no_enrichment_long_input() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );

        let long_text = "a".repeat(100);
        let enriched = enrich_with_context(&long_text, &ctx);
        assert_eq!(enriched.text, long_text);
        assert_eq!(enriched.reason, EnrichmentReason::None);
    }

    #[test]
    fn no_enrichment_stale_context() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );
        // Manually set last_turn_at to 120s ago to simulate stale context
        ctx.last_turn_at = Some(Instant::now() - Duration::from_secs(120));

        let enriched = enrich_with_context("check disk", &ctx);
        assert_eq!(enriched.reason, EnrichmentReason::None);
    }

    #[test]
    fn no_enrichment_empty_context() {
        let ctx = RoutingContext::default();
        let enriched = enrich_with_context("open Chrome", &ctx);
        assert_eq!(enriched.text, "open Chrome");
        assert_eq!(enriched.reason, EnrichmentReason::None);
    }

    #[test]
    fn no_enrichment_very_short_input() {
        let mut ctx = RoutingContext::default();
        ctx.record_turn(
            Domain::SystemInfo,
            None,
            IntentModality::Read,
            vec![0.1; 10],
        );

        let enriched = enrich_with_context("ab", &ctx); // 2 chars < MIN_ENRICH_LENGTH
        assert_eq!(enriched.text, "ab");
        assert_eq!(enriched.reason, EnrichmentReason::None);
    }

    #[test]
    fn serialization_roundtrip() {
        let ctx = RoutingContext {
            last_domain: Some(Domain::FileOps),
            last_tool: Some("read_file".into()),
            turn_count_in_topic: 3,
            ..Default::default()
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let restored: RoutingContext = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.last_domain, Some(Domain::FileOps));
        assert_eq!(restored.turn_count_in_topic, 3);
    }

    #[test]
    fn latency_under_budget() {
        let ctx = RoutingContext::default();
        let start = Instant::now();
        for _ in 0..1000 {
            let _ = enrich_with_context("check system status", &ctx);
        }
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50)); // 1000 enrichments < 50ms
    }
}
