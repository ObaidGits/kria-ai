//! Shadow comparator (HRA Task 37 / R22.3).
//!
//! Replays an identical device snapshot to both the legacy static path and the RA Planner, then
//! asserts the RA never violates safety invariants the legacy path upheld. Produces a divergence
//! report that gates cutover: cutover is allowed only when there are zero invariant violations over
//! the soak.

use super::device_table::DeviceTable;
use super::planner::{self, PolicyProfile};
use super::types::{DeviceId, PrivacyReq, Residency, ResourceRequest};

/// Legacy static placement (the pre-RA behavior): full GPU0 if it fits, else CPU.
fn legacy_plan(req: &ResourceRequest, table: &DeviceTable) -> DeviceId {
    table
        .usable_gpus()
        .into_iter()
        .find(|d| d.can_admit_vram(req.need.vram_mb))
        .map(|d| d.id.clone())
        .unwrap_or(DeviceId::Cpu)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    pub turn: String,
    pub legacy_device: DeviceId,
    pub ra_device: DeviceId,
    /// True when the devices differ (informational, not necessarily a violation).
    pub differs: bool,
    /// Invariant violations (these gate cutover).
    pub violations: Vec<String>,
}

/// Compare one request. Invariants checked on the RA plan:
/// - feasibility: a GPU choice must be admissible (no over-commit, Property 1).
/// - privacy: a privacy-strict request must NEVER be placed on cloud (Property 13).
pub fn compare(req: &ResourceRequest, table: &DeviceTable, profile: PolicyProfile) -> Divergence {
    let legacy_device = legacy_plan(req, table);
    let ra_plan = planner::plan(req, table, profile);
    let ra_device = ra_plan.device.clone();

    let mut violations = Vec::new();

    // Over-commit invariant: if RA chose a GPU, it must be admissible.
    if let DeviceId::Gpu(_) = &ra_device {
        if let Some(rec) = table.get(&ra_device) {
            if !rec.can_admit_vram(req.need.vram_mb) {
                violations.push(format!(
                    "RA planned GPU {:?} that cannot admit {} MB (over-commit)",
                    ra_device, req.need.vram_mb
                ));
            }
        }
    }

    // Privacy invariant.
    if req.constraints.privacy == PrivacyReq::Strict && matches!(ra_device, DeviceId::CloudPool(_))
    {
        violations.push("RA planned cloud for a privacy-strict request".into());
    }

    // Residency sanity.
    if matches!(ra_device, DeviceId::Cpu) && ra_plan.residency == Residency::VramHot {
        violations.push("RA planned CPU device with VramHot residency".into());
    }

    Divergence {
        turn: req.turn_id.0.clone(),
        legacy_device: legacy_device.clone(),
        ra_device,
        differs: legacy_plan(req, table) != ra_plan.device,
        violations,
    }
}

/// Aggregate report over a soak window.
#[derive(Debug, Clone, Default)]
pub struct ShadowReport {
    pub samples: usize,
    pub diffs: usize,
    pub violations: Vec<String>,
}

impl ShadowReport {
    pub fn record(&mut self, d: Divergence) {
        self.samples += 1;
        if d.differs {
            self.diffs += 1;
        }
        self.violations.extend(d.violations);
    }

    /// Cutover gate: pass only when no invariant was ever violated.
    pub fn gate_passes(&self) -> bool {
        self.violations.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::super::device_table::{DeviceRecord, DeviceTable};
    use super::super::types::{Constraints, ConsumerId, PriorityClass, ResourceNeed, TurnId};
    use super::*;

    fn req(vram: u64, privacy: PrivacyReq) -> ResourceRequest {
        ResourceRequest {
            consumer: ConsumerId::Llm,
            class: PriorityClass::InteractiveFg,
            need: ResourceNeed {
                vram_mb: vram,
                ram_mb: 2048,
                cpu_threads: 4,
                exclusivity: false,
                model_id: None,
                est_ms: 1000,
            },
            constraints: Constraints {
                privacy,
                ..Default::default()
            },
            turn_id: TurnId("t".into()),
        }
    }

    fn table() -> DeviceTable {
        let mut t = DeviceTable::new();
        t.upsert(DeviceRecord::gpu(0, 12288, 512));
        t.upsert(DeviceRecord::cpu(32768));
        t.upsert(DeviceRecord::cloud("openai"));
        t
    }

    #[test]
    fn no_violations_on_normal_request() {
        let d = compare(
            &req(4000, PrivacyReq::Standard),
            &table(),
            PolicyProfile::Balanced,
        );
        assert!(d.violations.is_empty());
    }

    #[test]
    fn privacy_strict_never_cloud_holds() {
        // Even with no fitting GPU, RA must not pick cloud for strict.
        let mut t = DeviceTable::new();
        t.upsert(DeviceRecord::gpu(0, 2048, 512));
        t.upsert(DeviceRecord::cpu(16384));
        t.upsert(DeviceRecord::cloud("openai"));
        let d = compare(&req(8000, PrivacyReq::Strict), &t, PolicyProfile::Balanced);
        assert!(d.violations.is_empty());
        assert_eq!(d.ra_device, DeviceId::Cpu);
    }

    #[test]
    fn report_gates_on_zero_violations() {
        let t = table();
        let mut r = ShadowReport::default();
        for _ in 0..20 {
            r.record(compare(
                &req(4000, PrivacyReq::Standard),
                &t,
                PolicyProfile::Balanced,
            ));
        }
        assert!(r.gate_passes());
        assert_eq!(r.samples, 20);
    }
}
