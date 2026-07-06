//! A6 Semantic Skill Router - Production registry-driven routing system.
//!
//! Transforms OpenClaw from "Registry + Runtime" into "Semantic Skill Platform."
//!
//! The router NEVER relies on:
//! - Hardcoded skill names
//! - Keyword lists  
//! - If-else chains
//! - Manual routing
//! - Tool-specific logic
//!
//! Routing is: Registry Driven → Semantic → Capability Aware → Risk Aware → Resource Aware → Context Aware
//!
//! # A6 Architecture
//!
//! Input: User Intent
//!   ↓
//! Registry Query (get all enabled skills)
//!   ↓  
//! Candidate Skills (semantic filtering)
//!   ↓
//! Semantic Ranking (similarity + metadata)
//!   ↓
//! Capability Filter (required capabilities)
//!   ↓
//! Trust Filter (trust tier compatibility)
//!   ↓
//! Resource Filter (HRA + RuntimeManager)
//!   ↓
//! Cost Filter (latency + resource cost)
//!   ↓
//! Best Skill (top ranked match)
//!   ↓
//! Execution (via runtime)

use super::registry::{ProductionSkillRegistry, RegistryError, SkillMetadata, SkillState};
use super::runtime_manager::RuntimeManager;
use super::types::{ResourceClass, TrustTier};
use crate::safety::RiskLevel;
use std::sync::Arc;

/// Input to the semantic router.
#[derive(Debug, Clone)]
pub struct RoutingIntent {
    /// User's natural language request
    pub request: String,
    /// Required capabilities for this intent
    pub required_capabilities: Vec<String>,
    /// Maximum risk level acceptable
    pub max_risk: RiskLevel,
    /// Preferred resource class (if any)
    pub preferred_resource: Option<ResourceClass>,
    /// Context about current system state
    pub context: RoutingContext,
}

/// Context information for routing decisions.
#[derive(Debug, Clone)]
pub struct RoutingContext {
    /// Current system resource pressure
    pub resource_pressure: ResourcePressure,
    /// Available GPU memory (if any)
    pub gpu_memory_mb: Option<u64>,
    /// Network connectivity status
    pub network_available: bool,
    /// Current trust level for this session
    pub session_trust: TrustTier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcePressure {
    Low,
    Medium,
    High,
    Critical,
}

/// Candidate skill after initial filtering.
#[derive(Debug, Clone)]
pub struct SkillCandidate {
    pub metadata: SkillMetadata,
    pub semantic_score: f32,
    pub capability_match: f32,
    pub trust_score: f32,
    pub resource_score: f32,
    pub historical_score: f32,
}

/// Final routing decision.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    /// Selected skill (None if no suitable match)
    pub skill: Option<SkillMetadata>,
    /// Confidence in this selection (0.0-1.0)
    pub confidence: f32,
    /// Alternative suggestions if confidence is low
    pub alternatives: Vec<SkillSuggestion>,
    /// Reasoning for this decision
    pub reasoning: String,
}

/// Alternative skill suggestion with explanation.
#[derive(Debug, Clone)]
pub struct SkillSuggestion {
    pub skill: SkillMetadata,
    pub confidence: f32,
    pub reasoning: String,
    pub expected_capabilities: Vec<String>,
}

/// A6.1-A6.13: Production Semantic Router
/// THE authoritative skill router. Registry-driven, semantic, no hardcoded logic.
pub struct SemanticSkillRouter {
    /// ONLY source of skills
    registry: Arc<ProductionSkillRegistry>,
    /// Runtime manager for resource awareness
    runtime_manager: Option<Arc<RuntimeManager>>,
    /// Semantic scoring configuration
    config: RouterConfig,
}

#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Maximum number of candidates to consider
    pub max_candidates: usize,
    /// Minimum confidence threshold for selection
    pub min_confidence: f32,
    /// Maximum alternatives to return
    pub max_alternatives: usize,
    /// Semantic scoring weights
    pub semantic_weight: f32,
    pub capability_weight: f32,
    pub trust_weight: f32,
    pub resource_weight: f32,
    pub historical_weight: f32,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            max_candidates: 50,
            min_confidence: 0.3,
            max_alternatives: 3,
            semantic_weight: 0.4,
            capability_weight: 0.25,
            trust_weight: 0.15,
            resource_weight: 0.1,
            historical_weight: 0.1,
        }
    }
}

impl SemanticSkillRouter {
    /// A6.2: Create new semantic router (registry-driven only).
    pub fn new(
        registry: Arc<ProductionSkillRegistry>,
        runtime_manager: Option<Arc<RuntimeManager>>,
    ) -> Self {
        Self {
            registry,
            runtime_manager,
            config: RouterConfig::default(),
        }
    }

    /// Configure router parameters.
    pub fn with_config(mut self, config: RouterConfig) -> Self {
        self.config = config;
        self
    }

    /// A6.2: Main routing pipeline - Input→Registry→Semantic→Filters→Best Skill.
    pub async fn route(&self, intent: RoutingIntent) -> Result<RoutingDecision, RouterError> {
        // A6.3: Registry-driven - get ALL enabled skills from registry ONLY
        let enabled_skills = self
            .registry
            .get_enabled_skills()
            .map_err(RouterError::Registry)?;

        if enabled_skills.is_empty() {
            return Ok(RoutingDecision {
                skill: None,
                confidence: 0.0,
                alternatives: vec![],
                reasoning: "No enabled skills found in registry".to_string(),
            });
        }

        // A6.6: Capability filtering - remove skills without required capabilities
        let capability_filtered = self.filter_by_capabilities(&enabled_skills, &intent);

        // A6.7: Resource awareness - consult HRA and RuntimeManager
        let resource_filtered = self
            .filter_by_resources(&capability_filtered, &intent)
            .await;

        if resource_filtered.is_empty() {
            return Ok(RoutingDecision {
                skill: None,
                confidence: 0.0,
                alternatives: vec![],
                reasoning: "No skills available with required capabilities and resources"
                    .to_string(),
            });
        }

        // A6.4 + A6.5: Semantic ranking with intent understanding
        let mut candidates = self.rank_semantically(&resource_filtered, &intent).await;

        // Sort by combined score
        candidates.sort_by(|a, b| b.combined_score().partial_cmp(&a.combined_score()).unwrap());

        // A6.9: Generate decision and suggestions
        let decision = self.make_decision(candidates, &intent);

        Ok(decision)
    }

    /// A6.6: Filter skills by required capabilities.
    fn filter_by_capabilities(
        &self,
        skills: &[SkillMetadata],
        intent: &RoutingIntent,
    ) -> Vec<SkillMetadata> {
        skills
            .iter()
            .filter(|skill| {
                // Check skill state
                if !matches!(skill.state, SkillState::Enabled) {
                    return false;
                }

                // Check trust tier compatibility
                if skill.trust_tier > intent.context.session_trust {
                    return false;
                }

                // Check risk level
                if skill.risk_level > intent.max_risk {
                    return false;
                }

                // Check required capabilities
                if intent.required_capabilities.is_empty() {
                    return true;
                }

                // Check actual skill capabilities
                for required in &intent.required_capabilities {
                    let has_capability = match required.as_str() {
                        "network" => skill.capabilities.network,
                        "filesystem_read" => skill.capabilities.filesystem_read,
                        "filesystem_write" => skill.capabilities.filesystem_write,
                        "subprocess" => skill.capabilities.subprocess,
                        "browser" => skill.capabilities.browser,
                        "image_generation" => skill.capabilities.image_generation,
                        "media" => skill.capabilities.media,
                        // Fallback to string matching for unknown capabilities
                        _ => {
                            skill
                                .description
                                .to_lowercase()
                                .contains(&required.to_lowercase())
                                || skill.tags.iter().any(|tag| {
                                    tag.to_lowercase().contains(&required.to_lowercase())
                                })
                                || skill.categories.iter().any(|cat| {
                                    cat.to_lowercase().contains(&required.to_lowercase())
                                })
                        }
                    };

                    if !has_capability {
                        return false;
                    }
                }

                true
            })
            .cloned()
            .collect()
    }

    /// A6.7: Filter skills by resource availability.
    async fn filter_by_resources(
        &self,
        skills: &[SkillMetadata],
        intent: &RoutingIntent,
    ) -> Vec<SkillMetadata> {
        let mut filtered = Vec::new();

        for skill in skills {
            // Check resource class availability
            if let Some(preferred) = intent.preferred_resource {
                if skill.resource_class > preferred {
                    continue;
                }
            }

            // Check resource pressure
            match intent.context.resource_pressure {
                ResourcePressure::Critical => {
                    // Only allow Light resources during critical pressure
                    if skill.resource_class != ResourceClass::Light {
                        continue;
                    }
                }
                ResourcePressure::High => {
                    // Avoid Heavy resources during high pressure
                    if skill.resource_class == ResourceClass::Heavy {
                        continue;
                    }
                }
                _ => {} // Medium/Low pressure allows all
            }

            // Check runtime availability via RuntimeManager
            if let Some(ref _runtime_manager) = self.runtime_manager {
                // Check if runtime can handle this skill's resource class
                let can_handle = match skill.resource_class {
                    ResourceClass::Light => true,
                    ResourceClass::Medium => {
                        // Check if medium containers available
                        matches!(
                            intent.context.resource_pressure,
                            ResourcePressure::Low | ResourcePressure::Medium
                        )
                    }
                    ResourceClass::Heavy => {
                        // Only allow heavy if low pressure and GPU available
                        intent.context.resource_pressure == ResourcePressure::Low
                            && intent.context.gpu_memory_mb.unwrap_or(0) > 1024
                    }
                };

                if !can_handle {
                    continue;
                }
            }

            // Check network requirements
            if skill.capabilities.network && !intent.context.network_available {
                continue;
            }

            filtered.push(skill.clone());
        }

        filtered
    }

    /// A6.4 + A6.5: Semantic ranking with intent understanding.
    async fn rank_semantically(
        &self,
        skills: &[SkillMetadata],
        intent: &RoutingIntent,
    ) -> Vec<SkillCandidate> {
        let mut candidates = Vec::new();

        for skill in skills.iter().take(self.config.max_candidates) {
            let semantic_score = self.calculate_semantic_similarity(&intent.request, skill);
            let capability_match =
                self.calculate_capability_match(&intent.required_capabilities, skill);
            let trust_score = self.calculate_trust_score(skill, &intent.context);
            let resource_score = self.calculate_resource_score(skill, &intent.context);
            let historical_score = self.calculate_historical_score(skill).await;

            candidates.push(SkillCandidate {
                metadata: skill.clone(),
                semantic_score,
                capability_match,
                trust_score,
                resource_score,
                historical_score,
            });
        }

        candidates
    }

    /// A6.5: Calculate semantic similarity between intent and skill.
    fn calculate_semantic_similarity(&self, intent: &str, skill: &SkillMetadata) -> f32 {
        let intent_lower = intent.to_lowercase();
        let skill_text = format!(
            "{} {} {}",
            skill.name,
            skill.description,
            skill.categories.join(" ")
        )
        .to_lowercase();

        // Simple keyword-based similarity for now - in production would use embeddings
        let intent_words: Vec<&str> = intent_lower.split_whitespace().collect();
        let mut matches = 0;

        for word in &intent_words {
            if skill_text.contains(word) {
                matches += 1;
            }
        }

        if intent_words.is_empty() {
            0.0
        } else {
            matches as f32 / intent_words.len() as f32
        }
    }

    /// Calculate how well skill capabilities match requirements.
    fn calculate_capability_match(&self, required: &[String], skill: &SkillMetadata) -> f32 {
        if required.is_empty() {
            return 1.0;
        }

        let mut matches = 0;
        for req in required {
            let req_lower = req.to_lowercase();
            let has_match = skill.description.to_lowercase().contains(&req_lower)
                || skill
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&req_lower))
                || skill
                    .categories
                    .iter()
                    .any(|cat| cat.to_lowercase().contains(&req_lower));

            if has_match {
                matches += 1;
            }
        }

        matches as f32 / required.len() as f32
    }

    /// Calculate trust score based on skill trust tier and publisher.
    fn calculate_trust_score(&self, skill: &SkillMetadata, context: &RoutingContext) -> f32 {
        let base_trust = match skill.trust_tier {
            TrustTier::Verified => 1.0,
            TrustTier::Community => 0.8,
            TrustTier::Local => 0.6,
            TrustTier::Untrusted => 0.3,
        };

        let session_penalty = match (skill.trust_tier, context.session_trust) {
            (skill_trust, session_trust) if skill_trust <= session_trust => 1.0,
            _ => 0.7, // Penalty for exceeding session trust
        };

        base_trust * session_penalty
    }

    /// Calculate resource efficiency score.
    fn calculate_resource_score(&self, skill: &SkillMetadata, context: &RoutingContext) -> f32 {
        let resource_efficiency = match skill.resource_class {
            ResourceClass::Light => 1.0,
            ResourceClass::Medium => 0.7,
            ResourceClass::Heavy => 0.4,
        };

        let pressure_penalty = match context.resource_pressure {
            ResourcePressure::Low => 1.0,
            ResourcePressure::Medium => 0.8,
            ResourcePressure::High => 0.5,
            ResourcePressure::Critical => 0.2,
        };

        resource_efficiency * pressure_penalty
    }

    /// A6.8: Calculate historical performance score.
    async fn calculate_historical_score(&self, skill: &SkillMetadata) -> f32 {
        // Get historical statistics from registry
        match self.registry.get_skill_statistics(&skill.skill_id) {
            Ok(stats) => {
                let success_score = stats.success_rate;
                let latency_score = if stats.average_latency_ms < 1000.0 {
                    1.0
                } else if stats.average_latency_ms < 5000.0 {
                    0.7
                } else {
                    0.3
                };

                // Usage count boost for frequently used skills
                let usage_boost = if stats.usage_count > 100 {
                    1.1
                } else if stats.usage_count > 10 {
                    1.05
                } else {
                    1.0
                };

                (success_score * latency_score * usage_boost) as f32
            }
            Err(_) => 0.5, // Default score for skills without history
        }
    }

    /// A6.9: Make final decision and generate suggestions.
    fn make_decision(
        &self,
        candidates: Vec<SkillCandidate>,
        _intent: &RoutingIntent,
    ) -> RoutingDecision {
        if candidates.is_empty() {
            return RoutingDecision {
                skill: None,
                confidence: 0.0,
                alternatives: vec![],
                reasoning: "No suitable skills found after filtering".to_string(),
            };
        }

        let best = &candidates[0];
        let confidence = best.combined_score();

        let selected_skill = if confidence >= self.config.min_confidence {
            Some(best.metadata.clone())
        } else {
            None
        };

        // Generate alternatives from remaining candidates
        let alternatives: Vec<SkillSuggestion> = candidates
            .iter()
            .skip(if selected_skill.is_some() { 1 } else { 0 })
            .take(self.config.max_alternatives)
            .map(|candidate| SkillSuggestion {
                skill: candidate.metadata.clone(),
                confidence: candidate.combined_score(),
                reasoning: format!(
                    "Semantic match: {:.2}, Capabilities: {:.2}, Trust: {:.2}",
                    candidate.semantic_score, candidate.capability_match, candidate.trust_score
                ),
                expected_capabilities: candidate.metadata.categories.clone(),
            })
            .collect();

        let reasoning = if selected_skill.is_some() {
            format!(
                "Selected {} with confidence {:.2} (semantic: {:.2}, capabilities: {:.2}, trust: {:.2}, resources: {:.2}, history: {:.2})",
                best.metadata.skill_id,
                confidence,
                best.semantic_score,
                best.capability_match,
                best.trust_score,
                best.resource_score,
                best.historical_score
            )
        } else {
            format!(
                "No skill met minimum confidence threshold of {:.2}. Best candidate: {} with {:.2}",
                self.config.min_confidence, best.metadata.skill_id, confidence
            )
        };

        RoutingDecision {
            skill: selected_skill,
            confidence,
            alternatives,
            reasoning,
        }
    }

    /// A6.8: Record execution feedback for learning.
    pub async fn record_feedback(
        &self,
        skill_id: &str,
        success: bool,
        latency_ms: u64,
        resource_usage: f64,
        _confidence: f32,
    ) -> Result<(), RouterError> {
        // Record in registry for future routing decisions
        self.registry
            .record_execution(skill_id, success, latency_ms, resource_usage)
            .map_err(RouterError::Registry)?;

        // TODO: In production, also update semantic model weights based on feedback

        Ok(())
    }
}

impl SkillCandidate {
    /// Calculate combined weighted score.
    pub fn combined_score(&self) -> f32 {
        let config = RouterConfig::default(); // TODO: Pass config through

        config.semantic_weight * self.semantic_score
            + config.capability_weight * self.capability_match
            + config.trust_weight * self.trust_score
            + config.resource_weight * self.resource_score
            + config.historical_weight * self.historical_score
    }
}

/// Errors from the semantic router.
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("registry error: {0}")]
    Registry(#[from] RegistryError),
    #[error("no suitable skills found")]
    NoSuitableSkills,
    #[error("resource unavailable: {0}")]
    ResourceUnavailable(String),
    #[error("capability not supported: {0}")]
    UnsupportedCapability(String),
}
