//! BeliefGraph — Tracks current system state assumptions.
//!
//! # Deprecation Notice (Batch 1)
//!
//! `BeliefGraph` is **deprecated** in favor of `WorldModelStore` (accessed via
//! `PsdgHandle`). `WorldModelStore` provides the same Bayesian (s,p,o) triple
//! semantics with persistence, FTS5, archive, and decay — all backed by SQLite.
//!
//! `BeliefGraph` is retained as an **in-memory fallback** only:
//! - Used by `UncertaintyEngine` when no `PsdgHandle` is attached.
//! - Should NOT receive new callers.
//! - Will be removed in Batch 2.
//!
//! Migrate: `BeliefGraph::update(prop, conf, ev, src)` →
//! `PsdgHandle::record_fact(subject, predicate, object, conf, src, ev)`
//!
//! # Design: Bayesian Belief Updates
//!
//! Each fact has a confidence score, evidence chain, and source.
//! Facts are updated when new evidence arrives using Bayesian update:
//! `new_confidence = 1 - (1 - old_confidence) * (1 - new_evidence_confidence)`
//!
//! Old facts decay in confidence over time (exponential decay).
//! This ensures the system doesn't trust stale information.

use std::collections::HashMap;
use std::time::Duration;

/// Source of a belief.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BeliefSource {
    /// Detected from system command output.
    Detected,
    /// User explicitly told us.
    UserStated,
    /// LLM reasoned about it.
    Inferred,
    /// Skill compiler output.
    Compiled,
}

/// A single fact in the belief graph.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BeliefFact {
    /// Human-readable proposition (e.g., "Nginx is running on VM1").
    pub proposition: String,
    /// Confidence in this fact (0.0 - 1.0).
    pub confidence: f64,
    /// Evidence supporting this fact (e.g., ["systemctl status nginx: active"]).
    pub evidence: Vec<String>,
    /// Source of this belief.
    pub source: BeliefSource,
    /// When this fact was last verified.
    pub last_verified_epoch: u64,
}

impl BeliefFact {
    /// Create a new belief fact.
    pub fn new(
        proposition: impl Into<String>,
        confidence: f64,
        evidence: impl Into<String>,
        source: BeliefSource,
    ) -> Self {
        Self {
            proposition: proposition.into(),
            confidence,
            evidence: vec![evidence.into()],
            source,
            last_verified_epoch: epoch_millis(),
        }
    }

    /// How old this fact is (in seconds).
    pub fn age_secs(&self) -> u64 {
        let now = epoch_millis();
        if now > self.last_verified_epoch {
            (now - self.last_verified_epoch) / 1000
        } else {
            0
        }
    }
}

/// BeliefGraph — tracks current system state assumptions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BeliefGraph {
    /// Facts indexed by proposition (for quick lookup).
    facts: HashMap<String, BeliefFact>,
    /// How fast confidence decays without re-verification (per hour).
    /// 0.05 = lose 5% confidence per hour.
    decay_rate_per_hour: f64,
}

impl BeliefGraph {
    /// Create a new empty belief graph.
    pub fn new() -> Self {
        Self {
            facts: HashMap::new(),
            decay_rate_per_hour: 0.05,
        }
    }

    /// Create with custom decay rate.
    pub fn with_decay_rate(decay_rate_per_hour: f64) -> Self {
        Self {
            facts: HashMap::new(),
            decay_rate_per_hour,
        }
    }

    /// Store or update a fact. If the fact already exists, update confidence
    /// using Bayesian update: `new = 1 - (1 - old) * (1 - evidence)`.
    pub fn update(
        &mut self,
        proposition: &str,
        confidence: f64,
        evidence: impl Into<String>,
        source: BeliefSource,
    ) {
        let evidence_str = evidence.into();

        if let Some(fact) = self.facts.get_mut(proposition) {
            // Bayesian update: combine old and new confidence
            // P(A|B) = 1 - (1-P(A)) * (1-P(B))
            let combined = 1.0 - (1.0 - fact.confidence) * (1.0 - confidence);
            fact.confidence = combined.clamp(0.0, 1.0);
            fact.evidence.push(evidence_str);
            fact.source = source;
            fact.last_verified_epoch = epoch_millis();
        } else {
            self.facts.insert(
                proposition.to_string(),
                BeliefFact::new(proposition, confidence, evidence_str, source),
            );
        }
    }

    /// Decay confidence of all facts based on time since last verification.
    /// Uses exponential decay: `confidence *= exp(-rate * hours)`.
    pub fn decay(&mut self) {
        let now = epoch_millis();
        for fact in self.facts.values_mut() {
            let hours = (now.saturating_sub(fact.last_verified_epoch)) as f64 / 3_600_000.0;
            fact.confidence *= (-self.decay_rate_per_hour * hours).exp();
            // Floor at 0.01 — never fully zero (prevents division issues)
            fact.confidence = fact.confidence.max(0.01);
        }
    }

    /// Get overall confidence for a set of propositions (geometric mean).
    /// If any fact is uncertain, overall confidence is low.
    pub fn confidence_for(&self, propositions: &[&str]) -> f64 {
        let relevant: Vec<f64> = self
            .facts
            .values()
            .filter(|f| propositions.iter().any(|p| f.proposition.contains(p)))
            .map(|f| f.confidence)
            .collect();

        if relevant.is_empty() {
            0.0 // No information = zero confidence
        } else {
            // Geometric mean — if any fact is uncertain, overall is uncertain
            let product: f64 = relevant.iter().product();
            product.powf(1.0 / relevant.len() as f64)
        }
    }

    /// Get a specific fact.
    pub fn get(&self, proposition: &str) -> Option<&BeliefFact> {
        self.facts.get(proposition)
    }

    /// Get all facts.
    pub fn all_facts(&self) -> &HashMap<String, BeliefFact> {
        &self.facts
    }

    /// Remove a fact.
    pub fn remove(&mut self, proposition: &str) -> Option<BeliefFact> {
        self.facts.remove(proposition)
    }

    /// Clear all facts.
    pub fn clear(&mut self) {
        self.facts.clear();
    }

    /// Number of facts in the graph.
    pub fn len(&self) -> usize {
        self.facts.len()
    }

    /// Check if the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// Get facts with confidence below a threshold (stale/uncertain).
    pub fn uncertain_facts(&self, threshold: f64) -> Vec<&BeliefFact> {
        self.facts
            .values()
            .filter(|f| f.confidence < threshold)
            .collect()
    }

    /// Get facts that haven't been verified recently.
    pub fn stale_facts(&self, max_age: Duration) -> Vec<&BeliefFact> {
        let max_age_secs = max_age.as_secs();
        self.facts
            .values()
            .filter(|f| f.age_secs() > max_age_secs)
            .collect()
    }
}

impl Default for BeliefGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Get current time in milliseconds since epoch.
fn epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_retrieves_facts() {
        let mut bg = BeliefGraph::new();
        bg.update(
            "Nginx is running",
            0.95,
            "systemctl status: active",
            BeliefSource::Detected,
        );

        let fact = bg.get("Nginx is running").unwrap();
        assert!((fact.confidence - 0.95).abs() < 0.001);
        assert_eq!(fact.source, BeliefSource::Detected);
        assert_eq!(fact.evidence.len(), 1);
    }

    #[test]
    fn bayesian_update_combines_confidence() {
        let mut bg = BeliefGraph::new();
        bg.update(
            "Nginx is running",
            0.8,
            "systemctl status: active",
            BeliefSource::Detected,
        );
        bg.update(
            "Nginx is running",
            0.9,
            "curl localhost: OK",
            BeliefSource::Detected,
        );

        let fact = bg.get("Nginx is running").unwrap();
        // Bayesian: 1 - (1-0.8) * (1-0.9) = 1 - 0.02 = 0.98
        assert!(
            (fact.confidence - 0.98).abs() < 0.01,
            "Bayesian update should give ~0.98, got {}",
            fact.confidence
        );
        assert_eq!(fact.evidence.len(), 2);
    }

    #[test]
    fn geometric_mean_confidence() {
        let mut bg = BeliefGraph::new();
        bg.update("Nginx is running", 0.9, "evidence1", BeliefSource::Detected);
        bg.update("Disk is OK", 0.8, "evidence2", BeliefSource::Detected);

        let conf = bg.confidence_for(&["Nginx", "Disk"]);
        // Geometric mean of 0.9 and 0.8 = sqrt(0.72) ≈ 0.849
        assert!(
            (conf - 0.849).abs() < 0.01,
            "Geometric mean should be ~0.849, got {}",
            conf
        );
    }

    #[test]
    fn confidence_for_missing_facts_returns_zero() {
        let bg = BeliefGraph::new();
        assert_eq!(bg.confidence_for(&["nonexistent"]), 0.0);
    }

    #[test]
    fn decay_reduces_confidence() {
        let mut bg = BeliefGraph::new();
        bg.update("Old fact", 0.9, "evidence", BeliefSource::Detected);

        // Manually set last_verified to 10 hours ago
        if let Some(fact) = bg.facts.get_mut("Old fact") {
            fact.last_verified_epoch = epoch_millis() - (10 * 3_600_000);
        }

        bg.decay();
        let fact = bg.get("Old fact").unwrap();
        // After 10 hours at 0.05/hour: 0.9 * exp(-0.05*10) = 0.9 * 0.607 ≈ 0.546
        assert!(
            fact.confidence < 0.6,
            "Confidence should decay to ~0.55, got {}",
            fact.confidence
        );
        assert!(
            fact.confidence > 0.5,
            "Confidence should not decay below 0.5, got {}",
            fact.confidence
        );
    }

    #[test]
    fn decay_never_goes_to_zero() {
        let mut bg = BeliefGraph::new();
        bg.update("Very old fact", 0.5, "evidence", BeliefSource::Detected);

        // Set to 1000 hours ago
        if let Some(fact) = bg.facts.get_mut("Very old fact") {
            fact.last_verified_epoch = epoch_millis() - (1000 * 3_600_000);
        }

        bg.decay();
        let fact = bg.get("Very old fact").unwrap();
        assert!(
            fact.confidence >= 0.01,
            "Confidence should never go below 0.01"
        );
    }

    #[test]
    fn uncertain_facts_filter() {
        let mut bg = BeliefGraph::new();
        bg.update("High confidence", 0.95, "evidence", BeliefSource::Detected);
        bg.update("Low confidence", 0.3, "evidence", BeliefSource::Inferred);

        let uncertain = bg.uncertain_facts(0.5);
        assert_eq!(uncertain.len(), 1);
        assert_eq!(uncertain[0].proposition, "Low confidence");
    }

    #[test]
    fn len_and_is_empty() {
        let mut bg = BeliefGraph::new();
        assert!(bg.is_empty());
        assert_eq!(bg.len(), 0);

        bg.update("Fact 1", 0.9, "evidence", BeliefSource::Detected);
        assert!(!bg.is_empty());
        assert_eq!(bg.len(), 1);
    }

    #[test]
    fn remove_fact() {
        let mut bg = BeliefGraph::new();
        bg.update("Fact to remove", 0.9, "evidence", BeliefSource::Detected);
        assert_eq!(bg.len(), 1);

        bg.remove("Fact to remove");
        assert!(bg.is_empty());
    }

    #[test]
    fn clear_all_facts() {
        let mut bg = BeliefGraph::new();
        bg.update("Fact 1", 0.9, "evidence", BeliefSource::Detected);
        bg.update("Fact 2", 0.8, "evidence", BeliefSource::Detected);
        assert_eq!(bg.len(), 2);

        bg.clear();
        assert!(bg.is_empty());
    }
}
