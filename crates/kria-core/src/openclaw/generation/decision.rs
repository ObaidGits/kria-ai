//! A9.0 Generation Decision Engine + A9.0.1 Similarity + A9.0.2 Policy.
//!
//! Generation is ALWAYS the last option: extract requirement → search the Production
//! Skill Registry for a suitable existing skill → reuse if similar enough → otherwise
//! generate. Never regenerate installed skills; always prefer reuse.

use super::requirements::SkillRequirement;
use serde::{Deserialize, Serialize};

/// A candidate existing skill to compare against (projected from the A5 registry).
#[derive(Debug, Clone)]
pub struct SkillCandidate {
    pub slug: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub capabilities: Vec<String>,
}

/// Weighted similarity breakdown (A9.0.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarityScore {
    pub semantic: f64,
    pub capability: f64,
    pub category: f64,
    pub tags: f64,
    pub overall: f64,
}

/// Generation policy (A9.0.2). Everything configurable — no hardcoded behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenerationPolicy {
    /// Always generate a fresh skill (ignore reuse).
    AlwaysGenerate,
    /// Generate only if no suitable existing skill (default).
    GenerateIfMissing,
    /// Defer to a human decision.
    AskUser,
    /// Site policy forbids generation.
    NeverGenerate,
}

impl Default for GenerationPolicy {
    fn default() -> Self {
        Self::GenerateIfMissing
    }
}

/// The decision the engine reached (A9.0).
#[derive(Debug, Clone, PartialEq)]
pub enum GenerationDecision {
    /// Reuse an existing skill (with its similarity score).
    Reuse { slug: String, similarity: f64 },
    /// Generate a new skill.
    Generate,
    /// A human must decide (AskUser policy with a close-but-not-certain match).
    AskUser {
        best_match: Option<String>,
        similarity: f64,
    },
    /// Policy forbids generation and nothing suitable exists.
    Denied,
}

/// The similarity engine (A9.0.1). Deterministic token/set-overlap scoring — no LLM.
pub struct SimilarityEngine;

impl SimilarityEngine {
    /// Score a requirement against a candidate skill.
    pub fn score(req: &SkillRequirement, cand: &SkillCandidate) -> SimilarityScore {
        let semantic = jaccard_tokens(&req.intent, &cand.description);
        let capability = jaccard_sets(&req.implied_capabilities, &cand.capabilities);
        let category = if req.category.eq_ignore_ascii_case(&cand.category) {
            1.0
        } else {
            0.0
        };
        let tags = jaccard_sets(&req.tags, &cand.tags);

        // Weighted overall — semantic + capability dominate.
        let overall = 0.5 * semantic + 0.3 * capability + 0.1 * category + 0.1 * tags;
        SimilarityScore {
            semantic,
            capability,
            category,
            tags,
            overall,
        }
    }

    /// Find the best-matching candidate, if any.
    pub fn best_match<'a>(
        req: &SkillRequirement,
        candidates: &'a [SkillCandidate],
    ) -> Option<(&'a SkillCandidate, SimilarityScore)> {
        candidates
            .iter()
            .map(|c| (c, Self::score(req, c)))
            .max_by(|a, b| {
                a.1.overall
                    .partial_cmp(&b.1.overall)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

/// The decision engine (A9.0 + A9.0.2). Config: reuse threshold + policy.
pub struct DecisionEngine {
    pub reuse_threshold: f64,
    pub policy: GenerationPolicy,
}

impl Default for DecisionEngine {
    fn default() -> Self {
        Self {
            reuse_threshold: 0.72,
            policy: GenerationPolicy::GenerateIfMissing,
        }
    }
}

impl DecisionEngine {
    pub fn new(reuse_threshold: f64, policy: GenerationPolicy) -> Self {
        Self {
            reuse_threshold,
            policy,
        }
    }

    /// Decide reuse vs generate for a requirement against installed candidates (A9.0).
    pub fn decide(
        &self,
        req: &SkillRequirement,
        candidates: &[SkillCandidate],
    ) -> GenerationDecision {
        let best = SimilarityEngine::best_match(req, candidates);
        let (best_slug, best_sim) = best
            .as_ref()
            .map(|(c, s)| (Some(c.slug.clone()), s.overall))
            .unwrap_or((None, 0.0));

        // Suitable existing skill → always prefer reuse (unless AlwaysGenerate).
        if best_sim >= self.reuse_threshold && self.policy != GenerationPolicy::AlwaysGenerate {
            if let Some(slug) = best_slug.clone() {
                return GenerationDecision::Reuse {
                    slug,
                    similarity: best_sim,
                };
            }
        }

        match self.policy {
            GenerationPolicy::AlwaysGenerate => GenerationDecision::Generate,
            GenerationPolicy::GenerateIfMissing => GenerationDecision::Generate,
            GenerationPolicy::NeverGenerate => GenerationDecision::Denied,
            GenerationPolicy::AskUser => GenerationDecision::AskUser {
                best_match: best_slug,
                similarity: best_sim,
            },
        }
    }
}

/// Jaccard similarity over whitespace tokens of two strings.
fn jaccard_tokens(a: &str, b: &str) -> f64 {
    let sa: std::collections::HashSet<String> = a
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let sb: std::collections::HashSet<String> = b
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    jaccard(&sa, &sb)
}

/// Jaccard similarity over two string sets.
fn jaccard_sets(a: &[String], b: &[String]) -> f64 {
    let sa: std::collections::HashSet<String> = a.iter().map(|s| s.to_lowercase()).collect();
    let sb: std::collections::HashSet<String> = b.iter().map(|s| s.to_lowercase()).collect();
    jaccard(&sa, &sb)
}

fn jaccard(a: &std::collections::HashSet<String>, b: &std::collections::HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}
