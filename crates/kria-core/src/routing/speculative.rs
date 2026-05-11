//! Speculative routing on partial voice transcripts.
//!
//! When a partial transcript arrives from the voice pipeline, this module
//! predicts the most likely routing path and pre-acquires resources
//! (GPU lease, tool serialization). If the prediction is wrong when the
//! final transcript arrives, the pre-warmed resources are cancelled.
//!
//! # Architecture
//!
//! ```text
//! Partial transcript (200ms into speech)
//!   → Fast embed + domain match
//!   → Predict: ResourcePlan (L1Text, ImageGpu, etc.)
//!   → Pre-acquire: GPU lease for predicted domain
//!   → Store: SpeculativeState
//!
//! Final transcript (500ms into speech)
//!   → Full routing decision
//!   → Compare with speculation
//!   → Hit:  Reuse pre-warmed GPU lease (save 200-400ms)
//!   → Miss: Release GPU, acquire fresh (no penalty)
//! ```
//!
//! # Latency Budget
//!
//! - Partial embed: ~10ms (fastembed single text)
//! - Domain match: <1ms (cosine similarity)
//! - GPU lease pre-acquire: ~50ms (async, non-blocking)
//! - Total speculation overhead: <60ms
//! - Latency saved on hit: 200-400ms

use std::time::{Duration, Instant};
use tracing::{debug, info};

use super::domain::Domain;
use super::embed;
use super::tool_index::ToolMatch;

// ─── Constants ──────────────────────────────────────────────────────────────

/// Minimum confidence to trigger speculation.
const DEFAULT_MIN_CONFIDENCE: f32 = 0.7;

/// Minimum tokens in partial transcript to trigger speculation.
const DEFAULT_MIN_TOKENS: usize = 2;

/// Maximum age of a speculation before it's considered stale.
const SPECULATION_TTL: Duration = Duration::from_secs(5);

// ─── Speculation Result ─────────────────────────────────────────────────────

/// Action to take based on partial transcript analysis.
#[derive(Debug, Clone, PartialEq)]
pub enum SpeculativeAction {
    /// Not enough information to speculate.
    Wait,
    /// Speculating — resources being pre-warmed.
    Speculating,
}

/// Result when final transcript arrives.
#[derive(Debug, Clone)]
pub enum SpeculativeResult {
    /// No speculation was active.
    NoSpeculation,
    /// Speculation matched the final routing decision.
    Hit {
        /// The pre-warmed state that was correct.
        prewarmed: SpeculativeState,
    },
    /// Speculation didn't match — resources cancelled.
    Miss {
        /// The wrong prediction that was made.
        predicted_domain: Option<Domain>,
        predicted_tool: Option<String>,
    },
}

// ─── Speculative State ──────────────────────────────────────────────────────

/// State maintained during a speculation.
#[derive(Debug, Clone)]
pub struct SpeculativeState {
    /// Predicted domain from partial transcript.
    pub predicted_domain: Domain,
    /// Predicted tool match (if any).
    pub predicted_tool: Option<ToolMatch>,
    /// Confidence of the prediction.
    pub confidence: f32,
    /// When speculation started.
    pub started_at: Instant,
    /// The partial text that triggered speculation.
    pub partial_text: String,
}

impl SpeculativeState {
    /// Check if this speculation has expired.
    pub fn is_expired(&self) -> bool {
        self.started_at.elapsed() > SPECULATION_TTL
    }

    /// Check if a new routing decision matches this speculation.
    pub fn matches(&self, domain: Domain, tool_name: Option<&str>) -> bool {
        if self.predicted_domain != domain {
            return false;
        }

        match (&self.predicted_tool, tool_name) {
            (Some(predicted), Some(actual)) => predicted.name == actual,
            (Some(_), None) => false, // predicted tool but none matched
            (None, _) => true,        // no tool prediction, domain match is enough
        }
    }
}

// ─── Speculative Router ─────────────────────────────────────────────────────

/// Speculative router that predicts routing from partial transcripts.
///
/// This is a lightweight, stateless predictor that runs on each partial
/// transcript. It uses the same embedding model as the main router but
/// with a faster, less accurate matching strategy.
pub struct SpeculativeRouter {
    /// Minimum confidence to trigger speculation.
    pub min_confidence: f32,
    /// Minimum tokens in partial text.
    pub min_tokens: usize,
    /// Active speculation state.
    pub active: Option<SpeculativeState>,
}

impl SpeculativeRouter {
    /// Create a new speculative router with default settings.
    pub fn new() -> Self {
        Self {
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            min_tokens: DEFAULT_MIN_TOKENS,
            active: None,
        }
    }

    /// Create with custom thresholds.
    pub fn with_thresholds(min_confidence: f32, min_tokens: usize) -> Self {
        Self {
            min_confidence,
            min_tokens,
            active: None,
        }
    }

    /// Process a partial transcript and decide whether to speculate.
    ///
    /// # Arguments
    ///
    /// * `text` - The partial transcript text
    /// * `confidence` - STT confidence (0.0-1.0)
    ///
    /// # Returns
    ///
    /// `SpeculativeAction::Speculating` if resources should be pre-warmed,
    /// `SpeculativeAction::Wait` otherwise.
    pub fn on_partial(&mut self, text: &str, confidence: f32) -> SpeculativeAction {
        // Gate 1: Minimum STT confidence
        if confidence < self.min_confidence {
            debug!(
                confidence,
                min = self.min_confidence,
                "Partial below confidence threshold — waiting"
            );
            return SpeculativeAction::Wait;
        }

        // Gate 2: Minimum token count
        let token_count = text.split_whitespace().count();
        if token_count < self.min_tokens {
            debug!(
                tokens = token_count,
                min = self.min_tokens,
                "Partial below token threshold — waiting"
            );
            return SpeculativeAction::Wait;
        }

        // Gate 3: Embedding model must be ready
        if !embed::is_ready() {
            return SpeculativeAction::Wait;
        }

        // Gate 4: Don't re-speculate if already speculating similar text
        if let Some(ref current) = self.active {
            if current.partial_text == text {
                return SpeculativeAction::Speculating;
            }
        }

        // Predict domain from partial text
        let prediction = self.predict_domain(text);

        match prediction {
            Some((domain, sim)) if sim >= self.min_confidence => {
                info!(
                    text = %text,
                    domain = ?domain,
                    confidence = sim,
                    "Speculative routing prediction"
                );

                self.active = Some(SpeculativeState {
                    predicted_domain: domain,
                    predicted_tool: None, // Tool prediction comes later with full routing
                    confidence: sim,
                    started_at: Instant::now(),
                    partial_text: text.to_string(),
                });

                SpeculativeAction::Speculating
            }
            _ => {
                debug!("No confident prediction from partial — waiting");
                SpeculativeAction::Wait
            }
        }
    }

    /// Process the final transcript and check against speculation.
    ///
    /// # Arguments
    ///
    /// * `final_text` - The final transcript text
    /// * `routed_domain` - The domain from full routing decision
    /// * `routed_tool` - The tool name from full routing (if any)
    ///
    /// # Returns
    ///
    /// `SpeculativeResult` indicating hit/miss/no-speculation.
    pub fn on_final(
        &mut self,
        _final_text: &str,
        routed_domain: Domain,
        routed_tool: Option<&str>,
    ) -> SpeculativeResult {
        let state = match self.active.take() {
            Some(s) => s,
            None => return SpeculativeResult::NoSpeculation,
        };

        // Check for expiry
        if state.is_expired() {
            debug!("Speculation expired — treating as miss");
            return SpeculativeResult::Miss {
                predicted_domain: Some(state.predicted_domain),
                predicted_tool: state.predicted_tool.as_ref().map(|t| t.name.clone()),
            };
        }

        // Check if prediction matches
        if state.matches(routed_domain, routed_tool) {
            info!(
                domain = ?routed_domain,
                confidence = state.confidence,
                latency_ms = state.started_at.elapsed().as_millis(),
                "Speculative routing HIT"
            );
            SpeculativeResult::Hit { prewarmed: state }
        } else {
            info!(
                predicted = ?state.predicted_domain,
                actual = ?routed_domain,
                latency_ms = state.started_at.elapsed().as_millis(),
                "Speculative routing MISS"
            );
            SpeculativeResult::Miss {
                predicted_domain: Some(state.predicted_domain),
                predicted_tool: state.predicted_tool.as_ref().map(|t| t.name.clone()),
            }
        }
    }

    /// Cancel any active speculation.
    pub fn cancel(&mut self) {
        if self.active.take().is_some() {
            debug!("Speculation cancelled");
        }
    }

    /// Check if there's an active speculation.
    pub fn has_active(&self) -> bool {
        self.active.is_some()
    }

    /// Get the active speculation state.
    pub fn active(&self) -> Option<&SpeculativeState> {
        self.active.as_ref()
    }

    /// Predict domain from partial text using fast embedding.
    ///
    /// Uses the same multilingual-e5-small model but with a simpler
    /// matching strategy (just top-1 domain, no OOD check).
    fn predict_domain(&self, text: &str) -> Option<(Domain, f32)> {
        let query_emb = embed::embed_one(text).ok()?;

        // Simple domain matching using anchor sentences
        // For speed, we compare against a small set of domain anchors
        let anchors = [
            (Domain::SystemInfo, "check system status hardware information CPU memory"),
            (Domain::FileOps, "read write copy move delete files folders"),
            (Domain::AppLifecycle, "open close launch start application window"),
            (Domain::Comms, "email send message calendar schedule reminder"),
            (Domain::Knowledge, "search web find information news weather"),
            (Domain::Power, "shutdown reboot volume brightness mute"),
            (Domain::Vision, "screenshot image analysis describe screen"),
            (Domain::Packages, "install uninstall update software package list installed apps"),
            (Domain::Developer, "run shell command git build lint"),
            (Domain::Workspace, "google docs drive sheets presentation"),
        ];

        // Embed all anchor phrases in one batch
        let anchor_texts: Vec<&str> = anchors.iter().map(|(_, t)| *t).collect();
        let anchor_embs = embed::embed_batch(&anchor_texts).ok()?;

        let mut best_domain = Domain::Conversation;
        let mut best_sim = 0.0f32;

        for (i, (domain, _)) in anchors.iter().enumerate() {
            let sim = embed::cosine_sim(&query_emb, &anchor_embs[i]);
            if sim > best_sim {
                best_sim = sim;
                best_domain = *domain;
            }
        }

        Some((best_domain, best_sim))
    }
}

impl Default for SpeculativeRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_thresholds() {
        let router = SpeculativeRouter::new();
        assert_eq!(router.min_confidence, DEFAULT_MIN_CONFIDENCE);
        assert_eq!(router.min_tokens, DEFAULT_MIN_TOKENS);
    }

    #[test]
    fn custom_thresholds() {
        let router = SpeculativeRouter::with_thresholds(0.8, 3);
        assert_eq!(router.min_confidence, 0.8);
        assert_eq!(router.min_tokens, 3);
    }

    #[test]
    fn low_confidence_waits() {
        let mut router = SpeculativeRouter::new();
        assert_eq!(
            router.on_partial("set volume", 0.3),
            SpeculativeAction::Wait
        );
    }

    #[test]
    fn short_text_waits() {
        let mut router = SpeculativeRouter::new();
        assert_eq!(
            router.on_partial("set", 0.9),
            SpeculativeAction::Wait
        );
    }

    #[test]
    fn no_active_by_default() {
        let router = SpeculativeRouter::new();
        assert!(!router.has_active());
        assert!(router.active().is_none());
    }

    #[test]
    fn cancel_clears_active() {
        let mut router = SpeculativeRouter::new();
        router.active = Some(SpeculativeState {
            predicted_domain: Domain::SystemInfo,
            predicted_tool: None,
            confidence: 0.9,
            started_at: Instant::now(),
            partial_text: "check system".into(),
        });
        assert!(router.has_active());
        router.cancel();
        assert!(!router.has_active());
    }

    #[test]
    fn on_final_no_speculation() {
        let mut router = SpeculativeRouter::new();
        let result = router.on_final("check system health", Domain::SystemInfo, None);
        assert!(matches!(result, SpeculativeResult::NoSpeculation));
    }

    #[test]
    fn state_matches_domain() {
        let state = SpeculativeState {
            predicted_domain: Domain::SystemInfo,
            predicted_tool: None,
            confidence: 0.9,
            started_at: Instant::now(),
            partial_text: "check system".into(),
        };
        assert!(state.matches(Domain::SystemInfo, None));
        assert!(!state.matches(Domain::FileOps, None));
    }

    #[test]
    fn state_matches_tool() {
        let state = SpeculativeState {
            predicted_domain: Domain::Power,
            predicted_tool: Some(ToolMatch {
                name: "set_volume".into(),
                description: "Set volume".into(),
                category: "power".into(),
                confidence: 0.9,
                direct_execution: true,
            }),
            confidence: 0.9,
            started_at: Instant::now(),
            partial_text: "set volume".into(),
        };
        assert!(state.matches(Domain::Power, Some("set_volume")));
        assert!(!state.matches(Domain::Power, Some("set_brightness")));
        assert!(!state.matches(Domain::SystemInfo, Some("set_volume")));
    }

    #[test]
    fn state_not_expired() {
        let state = SpeculativeState {
            predicted_domain: Domain::SystemInfo,
            predicted_tool: None,
            confidence: 0.9,
            started_at: Instant::now(),
            partial_text: "check".into(),
        };
        assert!(!state.is_expired());
    }

    #[test]
    fn state_expired() {
        let state = SpeculativeState {
            predicted_domain: Domain::SystemInfo,
            predicted_tool: None,
            confidence: 0.9,
            started_at: Instant::now() - Duration::from_secs(10),
            partial_text: "check".into(),
        };
        assert!(state.is_expired());
    }
}
