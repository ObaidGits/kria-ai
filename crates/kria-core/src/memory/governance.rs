//! Deterministic classification + scoring (memory-upgrade design §22, tasks 16/21).
//!
//! All functions are deterministic and LLM-free (L8). Importance is computed at
//! write time; an LLM may later nudge ambiguous cases (±2, task 21 full version).

use crate::memory::types::{EmphasisSignals, MemoryType, Source, StalenessClass};

/// Infer a memory type when the caller did not propose one. Deterministic
/// heuristics; the full classifier (task 21) may refine ambiguous cases.
pub fn classify_type(content: &str, proposed: Option<&MemoryType>, source: &Source) -> MemoryType {
    if let Some(t) = proposed {
        return t.clone();
    }
    let lower = content.to_lowercase();
    // Every invocation source's outcome is capability knowledge (design §46.1,
    // MGR-044 AC1: "native tool, MCP tool, OpenClaw skill, or sidecar task").
    // `Source::OpenClaw`/`Source::Sidecar` were added to this arm at task
    // F1.5.4: before that, every outcome writer uniformly tagged its source
    // `Source::Tool` (see `classify_tool_outcome_source`), so this arm's
    // narrower match was never actually exercised by an OpenClaw/sidecar
    // outcome in practice — now that the write path attributes its real
    // source, this classification must cover all four.
    if matches!(
        source,
        Source::Tool(_) | Source::Mcp { .. } | Source::OpenClaw(_) | Source::Sidecar(_)
    ) {
        return MemoryType::Capability;
    }
    if lower.contains("failed") || lower.contains("error") || lower.contains("could not") {
        return MemoryType::Failure;
    }
    if lower.starts_with("how to") || lower.contains("steps to") || lower.contains("workflow") {
        return MemoryType::Procedural;
    }
    MemoryType::Semantic
}

/// Default staleness class for a type (design §22.4). Governs re-verification.
pub fn default_staleness(memory_type: &MemoryType, has_verify_predicate: bool) -> StalenessClass {
    if has_verify_predicate {
        return StalenessClass::VolatileVerifiable;
    }
    match memory_type {
        MemoryType::WorldModel | MemoryType::Semantic => StalenessClass::Slow,
        MemoryType::UserProfile => StalenessClass::Slow,
        MemoryType::Procedural | MemoryType::Reflection => StalenessClass::Permanent,
        MemoryType::DesktopContext => StalenessClass::VolatileUnverifiable,
        _ => StalenessClass::Slow,
    }
}

/// Numerically-stable logistic function.
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Deterministic importance in [0, 10] (design §22.1). `novelty` is `1 -
/// max_similarity_to_existing` (from the dedup step; 1.0 when no similar memory).
pub fn score_importance(
    novelty: f32,
    source: &Source,
    emphasis: &EmphasisSignals,
    contradiction: bool,
) -> f32 {
    let authority = source.authority();
    let emphasis_score = {
        let mut s = 0.0f32;
        if emphasis.explicit_remember {
            s += 0.7;
        }
        s += (emphasis.repetition as f32 * 0.1).min(0.3);
        if !emphasis.marker_terms.is_empty() {
            s += 0.2;
        }
        s.min(1.0)
    };
    let surprise = if contradiction { 1.0 } else { 0.3 };
    let raw = 0.30 * novelty.clamp(0.0, 1.0)
        + 0.25 * 0.5 // goal_relevance placeholder (task 21 wires active goals)
        + 0.20 * authority
        + 0.15 * emphasis_score
        + 0.10 * surprise;
    // Center at 0.5 and sharpen so the [0,1] signal spreads across [0,10].
    10.0 * sigmoid(4.0 * (raw - 0.5))
}

/// Confidence from source reliability, dented by contradiction (design §22.5).
pub fn score_confidence(source: &Source, contradiction: bool) -> f32 {
    let base = source.authority();
    if contradiction {
        (base * 0.5).clamp(0.0, 1.0)
    } else {
        base.clamp(0.0, 1.0)
    }
}

/// Rough token estimate (~4 chars/token) for token-budget retrieval.
pub fn estimate_tokens(content: &str) -> u32 {
    ((content.len() as f32) / 4.0).ceil() as u32
}

/// First-person preference/identity phrasing that marks `content` as a durable
/// user-preference statement worth an eager `user_preference` fact write,
/// ahead of (and independent from) the slow-path LLM
/// [`crate::memory::semantic_parser::SemanticMemoryParser`] extraction. This is
/// the deterministic classification rule the desktop/server adapters
/// previously duplicated inline (task F1.5.2: adapters construct
/// caller/command only and carry no standalone domain-classification
/// decision) — LLM-free (L8), matching this module's other governance rules.
pub fn is_preference_statement(content: &str) -> bool {
    const PATTERNS: [&str; 10] = [
        "i prefer ",
        "i like ",
        "my name is ",
        "i am a ",
        "i work ",
        "i use ",
        "my favorite ",
        "i always ",
        "i never ",
        "i live ",
    ];
    let lower = content.to_lowercase();
    PATTERNS.iter().any(|p| lower.contains(p))
}

/// Quality filter: is this event noise not worth deriving a memory from?
/// (design §18.2 / R4). Deterministic.
pub fn is_noise(content: &str) -> bool {
    let t = content.trim();
    if t.len() < 3 {
        return true;
    }
    let lower = t.to_lowercase();
    // Transient/cancelled/debug chatter that carries no durable lesson.
    const NOISE: [&str; 6] = ["cancelled", "canceled", "retrying", "debug:", "ping", "ack"];
    NOISE.iter().any(|n| lower == *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn importance_rewards_user_emphasis() {
        let plain = score_importance(
            0.5,
            &Source::ExternalContent("web".into()),
            &EmphasisSignals::default(),
            false,
        );
        let emphasized = score_importance(
            1.0,
            &Source::User,
            &EmphasisSignals {
                explicit_remember: true,
                repetition: 2,
                marker_terms: vec!["important".into()],
            },
            false,
        );
        assert!(emphasized > plain);
        assert!((0.0..=10.0).contains(&emphasized));
    }

    #[test]
    fn contradiction_dents_confidence() {
        assert!(score_confidence(&Source::User, true) < score_confidence(&Source::User, false));
    }

    #[test]
    fn classify_defaults_and_infers() {
        assert_eq!(
            classify_type("random fact", None, &Source::User),
            MemoryType::Semantic
        );
        assert_eq!(
            classify_type("the build failed with an error", None, &Source::User),
            MemoryType::Failure
        );
        assert_eq!(
            classify_type("x", Some(&MemoryType::Goal), &Source::User),
            MemoryType::Goal
        );
    }

    #[test]
    fn noise_filter() {
        assert!(is_noise("cancelled"));
        assert!(is_noise("  "));
        assert!(!is_noise("the user prefers dark mode"));
    }

    #[test]
    fn preference_statement_detection() {
        assert!(is_preference_statement("I prefer dark mode"));
        assert!(is_preference_statement("My name is Alice"));
        assert!(is_preference_statement("I live in Berlin"));
        assert!(!is_preference_statement("the build failed with an error"));
        assert!(!is_preference_statement(""));
    }
}
