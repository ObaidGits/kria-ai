//! Capability Registry (HRA Task 46 / R28).
//!
//! Declarative table describing what each model can do, its quality tier, latency class, and
//! resource profile. The Planner selects models by a PURE, deterministic lookup/filter against
//! this registry — model selection is explainable and NEVER performed by an LLM (Property 19).

use serde::{Deserialize, Serialize};

use super::types::{ConsumerId, ResourceNeed};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityTier {
    Draft = 0,
    Standard = 1,
    High = 2,
    Max = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyClass {
    Realtime,
    Interactive,
    Batch,
}

/// One model's declared capabilities and resource profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapability {
    pub id: String,
    pub kind: ConsumerId,
    pub capabilities: Vec<String>,
    pub quality_tier: QualityTier,
    pub latency_class: LatencyClass,
    pub resource_profile: ResourceNeed,
    /// Whether this model runs locally (`true`) or via a cloud pool (`false`).
    pub local: bool,
}

/// Deterministic selection query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectQuery {
    pub kind: ConsumerId,
    pub required_capabilities: Vec<String>,
    pub min_quality: Option<QualityTier>,
    pub max_latency_class: Option<LatencyClass>,
    /// Hard VRAM ceiling the candidate must fit within (0 = no ceiling).
    pub vram_ceiling_mb: u64,
    /// Prefer local models first when true.
    pub prefer_local: bool,
}

/// In-memory registry. Populated from config + runtime discovery (reconciled at startup).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityRegistry {
    models: Vec<ModelCapability>,
}

impl CapabilityRegistry {
    pub fn new(models: Vec<ModelCapability>) -> Self {
        Self { models }
    }

    pub fn register(&mut self, model: ModelCapability) {
        // Replace by id to keep discovery idempotent.
        if let Some(slot) = self.models.iter_mut().find(|m| m.id == model.id) {
            *slot = model;
        } else {
            self.models.push(model);
        }
    }

    pub fn all(&self) -> &[ModelCapability] {
        &self.models
    }

    /// Deterministic selection: filter by hard constraints, then rank by a STABLE key
    /// (prefer-local, then higher quality, then tighter latency class, then smaller VRAM, then id).
    /// Returns the single best match or `None`. Same registry + query always yields the same result.
    pub fn select(&self, q: &SelectQuery) -> Option<&ModelCapability> {
        let lat_rank = |l: LatencyClass| match l {
            LatencyClass::Realtime => 0u8,
            LatencyClass::Interactive => 1,
            LatencyClass::Batch => 2,
        };

        let mut candidates: Vec<&ModelCapability> = self
            .models
            .iter()
            .filter(|m| m.kind == q.kind)
            .filter(|m| {
                q.required_capabilities
                    .iter()
                    .all(|req| m.capabilities.iter().any(|c| c == req))
            })
            .filter(|m| q.min_quality.map(|mq| m.quality_tier >= mq).unwrap_or(true))
            .filter(|m| {
                q.max_latency_class
                    .map(|ml| lat_rank(m.latency_class) <= lat_rank(ml))
                    .unwrap_or(true)
            })
            .filter(|m| q.vram_ceiling_mb == 0 || m.resource_profile.vram_mb <= q.vram_ceiling_mb)
            .collect();

        candidates.sort_by(|a, b| {
            // Stable, total ordering. Best first.
            let a_local = if q.prefer_local { !a.local } else { false };
            let b_local = if q.prefer_local { !b.local } else { false };
            a_local
                .cmp(&b_local) // false (local) sorts before true (cloud) when prefer_local
                .then(b.quality_tier.cmp(&a.quality_tier)) // higher quality first
                .then(lat_rank(a.latency_class).cmp(&lat_rank(b.latency_class))) // tighter latency first
                .then(a.resource_profile.vram_mb.cmp(&b.resource_profile.vram_mb)) // smaller footprint
                .then(a.id.cmp(&b.id)) // final tie-break: stable by id
        });

        candidates.into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn need(vram_mb: u64) -> ResourceNeed {
        ResourceNeed {
            vram_mb,
            ram_mb: 1024,
            cpu_threads: 2,
            exclusivity: false,
            model_id: None,
            est_ms: 500,
        }
    }

    fn llm(id: &str, q: QualityTier, lat: LatencyClass, vram: u64, local: bool) -> ModelCapability {
        ModelCapability {
            id: id.into(),
            kind: ConsumerId::Llm,
            capabilities: vec!["text".into()],
            quality_tier: q,
            latency_class: lat,
            resource_profile: need(vram),
            local,
        }
    }

    fn registry() -> CapabilityRegistry {
        CapabilityRegistry::new(vec![
            llm("local-high", QualityTier::High, LatencyClass::Interactive, 6000, true),
            llm("local-draft", QualityTier::Draft, LatencyClass::Realtime, 2000, true),
            llm("cloud-max", QualityTier::Max, LatencyClass::Batch, 0, false),
        ])
    }

    #[test]
    fn selection_is_deterministic() {
        let r = registry();
        let q = SelectQuery {
            kind: ConsumerId::Llm,
            required_capabilities: vec!["text".into()],
            min_quality: Some(QualityTier::Standard),
            max_latency_class: None,
            vram_ceiling_mb: 0,
            prefer_local: true,
        };
        let a = r.select(&q).map(|m| m.id.clone());
        let b = r.select(&q).map(|m| m.id.clone());
        assert_eq!(a, b);
        // prefer_local → local-high beats cloud-max despite lower quality.
        assert_eq!(a.as_deref(), Some("local-high"));
    }

    #[test]
    fn vram_ceiling_excludes_too_big() {
        let r = registry();
        let q = SelectQuery {
            kind: ConsumerId::Llm,
            required_capabilities: vec!["text".into()],
            min_quality: None,
            max_latency_class: None,
            vram_ceiling_mb: 3000, // only local-draft (2000) and cloud-max (0) qualify
            prefer_local: true,
        };
        assert_eq!(r.select(&q).unwrap().id, "local-draft");
    }

    #[test]
    fn no_match_returns_none() {
        let r = registry();
        let q = SelectQuery {
            kind: ConsumerId::Stt, // none registered
            required_capabilities: vec![],
            min_quality: None,
            max_latency_class: None,
            vram_ceiling_mb: 0,
            prefer_local: true,
        };
        assert!(r.select(&q).is_none());
    }

    #[test]
    fn register_is_idempotent_by_id() {
        let mut r = registry();
        let n = r.all().len();
        r.register(llm("local-high", QualityTier::Max, LatencyClass::Interactive, 6000, true));
        assert_eq!(r.all().len(), n); // replaced, not appended
        // capability updated
        let q = SelectQuery {
            kind: ConsumerId::Llm,
            required_capabilities: vec!["text".into()],
            min_quality: Some(QualityTier::Max),
            max_latency_class: Some(LatencyClass::Interactive),
            vram_ceiling_mb: 0,
            prefer_local: true,
        };
        assert_eq!(r.select(&q).unwrap().id, "local-high");
    }
}
