//! OpenClaw production validation & hardening harness (spec:
//! `openclaw-production-validation`).
//!
//! This module validates the FROZEN A0–A9 OpenClaw architecture end to end. It
//! introduces NO duplicate installer/registry/router/runtime/marketplace system —
//! every probe binds to the real `kria_core::openclaw::*` symbols. See
//! `.kiro/specs/openclaw-production-validation/design.md` for the full design.

pub mod a9_cloud_generation;
pub mod benchmark;
pub mod capability_classes;
pub mod concurrency_probe;
pub mod container_lifecycle;
pub mod engine_probe;
pub mod execute_e2e;
pub mod failure_campaign;
pub mod failure_injection;
pub mod fault_injector;
pub mod final_freeze;
pub mod fixtures;
pub mod freeze_report;
pub mod generated_vs_authored;
pub mod generation_e2e;
pub mod honesty_sweep;
pub mod icp_e2e;
pub mod installer_matrix;
pub mod leak_detector;
pub mod leak_freedom;
pub mod lifecycle;
pub mod live_marketplace;
pub mod marketplace;
pub mod performance_budgets;
pub mod pipeline_trace;
pub mod production_stress;
pub mod regression;
pub mod release_artifacts;
pub mod rig;
pub mod scale;
pub mod settings_surface;
pub mod skill_management;
pub mod soak;
pub mod stress;
pub mod trust_revocation;
pub mod ui_sync_probe;
pub mod upgrade;

// Added in later tasks per tasks.md (each `pub mod` lands with its task, never
// as an empty placeholder ahead of time):
// - engine_probe      (task 4)
// - pipeline_trace    (task 5)
// - installer_matrix  (task 8, extended task 7)
// - concurrency_probe (task 14)
// - ui_sync_probe     (task 16)
// - telemetry_assert  (task 17)
// - soak              (task 18)
// - upgrade           (task 19)
// - scale             (task 20)
// - benchmark         (task 23)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Which validation layer produced a given piece of evidence.
///
/// `Skipped` is NEVER treated as `Pass` for freeze purposes (design.md
/// "Freeze gate — evidence rule"); `Fixture` LLM evidence never counts toward
/// production readiness for A9 (design.md "Real-LLM policy").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layer {
    /// Layer 0 — CI-safe unit/integration, no Docker, fixtures + fakes.
    Ci,
    /// Layer 1 — test-rig integration, real Docker, local pinned image.
    Rig,
    /// Layer 2 — live gate, real desktop, OpenClaw enabled from the UI.
    Live,
    /// Layer 3 — failure injection over Layer 1.
    Fault,
    /// Layer 4 — soak, sustained Layer 1 workload.
    Soak,
    /// Layer Scale — large-marketplace / routing-under-scale.
    Scale,
    /// Layer 5 — production benchmark (mixed workload + faults + pressure).
    Benchmark,
}

/// LLM mode used to produce a piece of A9-generation evidence.
///
/// A `Fixture` result NEVER satisfies R5/R13 for a freeze verdict — only
/// `Real` (the actual configured local/cloud LLM backend) counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LlmMode {
    Fixture,
    Real,
}

/// Outcome of a single validation check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Pass,
    Fail,
    /// Skipped with an honest reason (e.g. Docker/desktop/LLM unavailable).
    /// `Skipped != Pass` — never counts toward the freeze verdict.
    Skipped(String),
}

impl Outcome {
    pub fn is_pass(&self) -> bool {
        matches!(self, Outcome::Pass)
    }
}

/// A single piece of validation evidence, tying a check to a requirement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRecord {
    /// Requirement id this record satisfies, e.g. "3.5".
    pub requirement: String,
    pub layer: Layer,
    /// Human label for the check, e.g. "drift_surfaced".
    pub name: String,
    pub outcome: Outcome,
    /// Durations, counts, container-baseline deltas, etc.
    pub metrics: HashMap<String, serde_json::Value>,
    /// Links this record to telemetry emitted by the real system (R17).
    pub correlation_id: Uuid,
    /// Log excerpts, `docker ps` snapshots, telemetry ids.
    pub evidence: Vec<String>,
    /// LLM mode, set only for A9-generation-related records.
    pub llm_mode: Option<LlmMode>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl EvidenceRecord {
    pub fn new(
        requirement: impl Into<String>,
        layer: Layer,
        name: impl Into<String>,
        outcome: Outcome,
    ) -> Self {
        Self {
            requirement: requirement.into(),
            layer,
            name: name.into(),
            outcome,
            metrics: HashMap::new(),
            correlation_id: Uuid::new_v4(),
            evidence: Vec::new(),
            llm_mode: None,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn with_metric(
        mut self,
        key: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        self.metrics.insert(key.into(), value.into());
        self
    }

    pub fn with_evidence(mut self, line: impl Into<String>) -> Self {
        self.evidence.push(line.into());
        self
    }

    pub fn with_llm_mode(mut self, mode: LlmMode) -> Self {
        self.llm_mode = Some(mode);
        self
    }

    /// Whether this record counts toward the freeze gate — `Skipped` and
    /// `Fixture`-LLM evidence never do (design.md freeze-gate evidence rule).
    pub fn counts_for_freeze(&self) -> bool {
        if !self.outcome.is_pass() {
            return false;
        }
        if let Some(LlmMode::Fixture) = self.llm_mode {
            return false;
        }
        true
    }
}

/// In-memory evidence store for a single validation run. `report.rs`
/// (extended, task 22) renders the freeze report bundle from this store.
#[derive(Debug, Default)]
pub struct EvidenceStore {
    records: Vec<EvidenceRecord>,
}

impl EvidenceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, record: EvidenceRecord) {
        self.records.push(record);
    }

    pub fn all(&self) -> &[EvidenceRecord] {
        &self.records
    }

    /// All records for a given requirement (e.g. "3" matches "3.1", "3.5", ...).
    pub fn for_requirement(&self, requirement_prefix: &str) -> Vec<&EvidenceRecord> {
        self.records
            .iter()
            .filter(|r| {
                r.requirement == requirement_prefix
                    || r.requirement.starts_with(&format!("{requirement_prefix}."))
            })
            .collect()
    }

    /// A requirement is satisfied if it has >=1 freeze-counting Pass and 0 Fail
    /// across all its recorded evidence (design.md: "every R1-R19 requirement to
    /// have >=1 Pass and 0 Fail... before emitting a frozen").
    pub fn requirement_satisfied(&self, requirement_prefix: &str) -> bool {
        let matching = self.for_requirement(requirement_prefix);
        if matching.is_empty() {
            return false;
        }
        let has_fail = matching.iter().any(|r| matches!(r.outcome, Outcome::Fail));
        let has_real_pass = matching.iter().any(|r| r.counts_for_freeze());
        has_real_pass && !has_fail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_never_counts_for_freeze() {
        let record = EvidenceRecord::new(
            "1.1",
            Layer::Live,
            "enable_ui",
            Outcome::Skipped("no desktop".into()),
        );
        assert!(!record.counts_for_freeze());
    }

    #[test]
    fn fixture_llm_never_counts_for_freeze() {
        let record = EvidenceRecord::new("5.1", Layer::Ci, "generation_pipeline", Outcome::Pass)
            .with_llm_mode(LlmMode::Fixture);
        assert!(!record.counts_for_freeze());
    }

    #[test]
    fn real_llm_pass_counts_for_freeze() {
        let record = EvidenceRecord::new("5.1", Layer::Live, "generation_pipeline", Outcome::Pass)
            .with_llm_mode(LlmMode::Real);
        assert!(record.counts_for_freeze());
    }

    #[test]
    fn requirement_satisfied_requires_pass_and_no_fail() {
        let mut store = EvidenceStore::new();
        store.record(EvidenceRecord::new(
            "2.1",
            Layer::Rig,
            "acquire_reuse",
            Outcome::Pass,
        ));
        assert!(store.requirement_satisfied("2"));

        store.record(EvidenceRecord::new(
            "2.2",
            Layer::Rig,
            "unhealthy_evict",
            Outcome::Fail,
        ));
        assert!(!store.requirement_satisfied("2"));
    }

    #[test]
    fn requirement_with_only_skipped_is_not_satisfied() {
        let mut store = EvidenceStore::new();
        store.record(EvidenceRecord::new(
            "1.1",
            Layer::Live,
            "enable_ui",
            Outcome::Skipped("no docker".into()),
        ));
        assert!(!store.requirement_satisfied("1"));
    }
}
