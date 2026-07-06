//! Planner (HRA Task 5 / R13.1).
//!
//! Pure, deterministic placement. `plan()` maps a `ResourceRequest` + `DeviceTable` + a
//! `PolicyProfile` to a `Plan` with an ordered `fallback_chain`. NO I/O, NO LLM, NO RNG — identical
//! inputs yield an identical plan (Property 3). Selection uses a transparent integer cost model so
//! it is fully explainable.
//!
//! Hard rules enforced here:
//! - Privacy-Strict requests NEVER receive a cloud plan; they fail to CPU (Property 13 / R23.2).
//! - A GPU candidate is only feasible if it can admit the need without breaching the hard limit
//!   (Property 1/18).
//! - If no GPU/cloud is feasible, the plan fails open to CPU (R1.5 / R12.3).

use super::device_table::DeviceTable;
use super::types::{
    Capacity, DeviceId, Plan, PrivacyReq, RationaleCode, Residency, ResourceRequest,
};

/// Deterministic cost weights. Higher weight = that factor matters more.
#[derive(Debug, Clone, Copy)]
pub struct PolicyWeights {
    pub w_latency: u64,
    pub w_cost: u64,
    pub w_power: u64,
    pub w_disrupt: u64,
}

/// Named profiles selected deterministically by machine state (AC/battery, thermal) + user choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyProfile {
    Balanced,
    Performance,
    BatterySaver,
    PrivacyStrict,
    ThermalCapped,
}

impl PolicyProfile {
    pub fn weights(&self) -> PolicyWeights {
        match self {
            // Latency matters most; cloud/cpu penalised vs local GPU.
            Self::Performance => PolicyWeights {
                w_latency: 4,
                w_cost: 1,
                w_power: 0,
                w_disrupt: 2,
            },
            Self::Balanced => PolicyWeights {
                w_latency: 2,
                w_cost: 2,
                w_power: 1,
                w_disrupt: 2,
            },
            // Battery: power dominates → prefer cloud/CPU over hot GPU.
            Self::BatterySaver => PolicyWeights {
                w_latency: 1,
                w_cost: 2,
                w_power: 4,
                w_disrupt: 1,
            },
            Self::PrivacyStrict => PolicyWeights {
                w_latency: 2,
                w_cost: 1,
                w_power: 1,
                w_disrupt: 2,
            },
            // Thermal capped: avoid sustained GPU → mild power/latency tradeoff.
            Self::ThermalCapped => PolicyWeights {
                w_latency: 2,
                w_cost: 1,
                w_power: 3,
                w_disrupt: 2,
            },
        }
    }
}

// Base per-placement penalties (pre-weight). Tuned so local GPU wins under Balanced/Performance.
const LAT_GPU: u64 = 0;
const LAT_CLOUD: u64 = 30;
const LAT_CPU: u64 = 100;

const POWER_GPU: u64 = 10;
const POWER_CLOUD: u64 = 1;
const POWER_CPU: u64 = 4;

const COST_GPU: u64 = 0;
const COST_CLOUD: u64 = 20;
const COST_CPU: u64 = 0;

/// A scored candidate (internal).
struct Candidate {
    plan: Plan,
    cost: u64,
    // deterministic tie-break key
    tiebreak: u64,
}

/// Produce a deterministic plan. `fallback_chain` holds the remaining feasible candidates in cost
/// order so the Scheduler can walk them if the primary fails.
pub fn plan(req: &ResourceRequest, table: &DeviceTable, profile: PolicyProfile) -> Plan {
    let w = profile.weights();
    let strict = req.constraints.privacy == PrivacyReq::Strict;
    let allow_cloud = req.constraints.allow_cloud && !strict;

    let mut candidates: Vec<Candidate> = Vec::new();

    // Extra GPU power penalty when the request explicitly asks for low power (battery).
    let gpu_power = POWER_GPU
        + if req.constraints.power == super::types::PowerReq::LowPower {
            40
        } else {
            0
        };

    // GPU candidates.
    for d in table.usable_gpus() {
        if !d.can_admit_vram(req.need.vram_mb) {
            continue;
        }
        let free_after = d.effective_free_vram_mb().saturating_sub(req.need.vram_mb);
        // Disruption: if the post-admission free would dip into the soft band, remedies may run.
        let disrupt = if d.budget.in_soft(free_after) { 1 } else { 0 };
        let rationale = if d.reserved_vram_mb > 0 {
            RationaleCode::CoResident
        } else {
            RationaleCode::FitsLocal
        };
        let cost = w.w_latency * LAT_GPU
            + w.w_cost * COST_GPU
            + w.w_power * gpu_power
            + w.w_disrupt * disrupt;
        candidates.push(Candidate {
            plan: Plan {
                device: d.id.clone(),
                residency: Residency::VramHot,
                budget: Capacity::vram(req.need.vram_mb),
                fallback_chain: Vec::new(),
                rationale,
            },
            cost,
            tiebreak: gpu_index(&d.id),
        });
    }

    // Cloud candidates (never for privacy-strict).
    if allow_cloud {
        for d in table.usable_cloud() {
            let cost = w.w_latency * LAT_CLOUD + w.w_cost * COST_CLOUD + w.w_power * POWER_CLOUD;
            candidates.push(Candidate {
                plan: Plan {
                    device: d.id.clone(),
                    residency: Residency::Cloud,
                    budget: Capacity::default(),
                    fallback_chain: Vec::new(),
                    rationale: RationaleCode::FailoverCloud,
                },
                cost,
                // cloud sorts after GPUs on ties via large tiebreak base
                tiebreak: 1_000 + pool_hash(&d.id),
            });
        }
    }

    // CPU fail-open candidate (always available).
    {
        let cost = w.w_latency * LAT_CPU + w.w_cost * COST_CPU + w.w_power * POWER_CPU;
        let rationale = if strict {
            RationaleCode::PrivacyLocalOnly
        } else {
            RationaleCode::FailOpenCpu
        };
        candidates.push(Candidate {
            plan: Plan {
                device: DeviceId::Cpu,
                residency: Residency::RamWarm,
                budget: Capacity {
                    ram_mb: req.need.ram_mb,
                    ..Default::default()
                },
                fallback_chain: Vec::new(),
                rationale,
            },
            cost,
            tiebreak: 2_000,
        });
    }

    // Deterministic sort: lowest cost first, then stable tiebreak.
    candidates.sort_by(|a, b| a.cost.cmp(&b.cost).then(a.tiebreak.cmp(&b.tiebreak)));

    // Best plan + ordered fallbacks.
    let mut iter = candidates.into_iter();
    let mut best = iter.next().expect("CPU candidate always present").plan;
    best.fallback_chain = iter.map(|c| c.plan).collect();
    best
}

fn gpu_index(id: &DeviceId) -> u64 {
    match id {
        DeviceId::Gpu(i) => *i as u64,
        _ => u64::MAX,
    }
}

fn pool_hash(id: &DeviceId) -> u64 {
    match id {
        DeviceId::CloudPool(name) => name.bytes().map(|b| b as u64).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::super::device_table::{DeviceRecord, DeviceTable};
    use super::super::types::{Constraints, ConsumerId, PriorityClass, ResourceNeed, TurnId};
    use super::*;

    fn req(vram_mb: u64, privacy: PrivacyReq, allow_cloud: bool) -> ResourceRequest {
        ResourceRequest {
            consumer: ConsumerId::Llm,
            class: PriorityClass::InteractiveFg,
            need: ResourceNeed {
                vram_mb,
                ram_mb: 2048,
                cpu_threads: 4,
                exclusivity: false,
                model_id: Some("m".into()),
                est_ms: 1000,
            },
            constraints: Constraints {
                privacy,
                allow_cloud,
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
    fn deterministic_same_inputs_same_plan() {
        let t = table();
        let r = req(4000, PrivacyReq::Standard, true);
        assert_eq!(
            plan(&r, &t, PolicyProfile::Balanced),
            plan(&r, &t, PolicyProfile::Balanced)
        );
    }

    #[test]
    fn balanced_prefers_local_gpu_when_it_fits() {
        let t = table();
        let p = plan(
            &req(4000, PrivacyReq::Standard, true),
            &t,
            PolicyProfile::Balanced,
        );
        assert_eq!(p.device, DeviceId::Gpu(0));
        assert_eq!(p.residency, Residency::VramHot);
        assert_eq!(p.rationale, RationaleCode::FitsLocal);
        // fallbacks present and ordered (cloud, cpu in some order by cost).
        assert!(!p.fallback_chain.is_empty());
    }

    #[test]
    fn privacy_strict_never_plans_cloud() {
        let t = table();
        let p = plan(
            &req(4000, PrivacyReq::Strict, true),
            &t,
            PolicyProfile::Balanced,
        );
        assert!(p.device != DeviceId::CloudPool("openai".into()));
        for fb in &p.fallback_chain {
            assert!(matches!(fb.device, DeviceId::Gpu(_) | DeviceId::Cpu));
        }
    }

    #[test]
    fn no_gpu_fits_falls_back_to_cloud_or_cpu() {
        let mut t = DeviceTable::new();
        // tiny GPU that cannot admit the need
        t.upsert(DeviceRecord::gpu(0, 2048, 512));
        t.upsert(DeviceRecord::cpu(16384));
        t.upsert(DeviceRecord::cloud("openai"));
        let p = plan(
            &req(8000, PrivacyReq::Standard, true),
            &t,
            PolicyProfile::Balanced,
        );
        assert!(matches!(p.device, DeviceId::CloudPool(_) | DeviceId::Cpu));
    }

    #[test]
    fn battery_saver_lowpower_deprioritizes_gpu() {
        let t = table();
        let mut r = req(4000, PrivacyReq::Standard, true);
        r.constraints.power = super::super::types::PowerReq::LowPower;
        let p = plan(&r, &t, PolicyProfile::BatterySaver);
        // Low-power request + battery profile → GPU power penalty dominates, cloud/CPU wins.
        assert!(matches!(p.device, DeviceId::CloudPool(_) | DeviceId::Cpu));
    }

    #[test]
    fn balanced_still_prefers_gpu_even_on_battery_profile_without_lowpower() {
        // Without an explicit low-power request, local GPU remains cheapest (correct: local is
        // free + low-latency; TPPE handles duty-cycling separately).
        let t = table();
        let p = plan(
            &req(4000, PrivacyReq::Standard, true),
            &t,
            PolicyProfile::BatterySaver,
        );
        assert_eq!(p.device, DeviceId::Gpu(0));
    }

    #[test]
    fn strict_with_no_fitting_gpu_fails_to_cpu_not_cloud() {
        let mut t = DeviceTable::new();
        t.upsert(DeviceRecord::gpu(0, 2048, 512)); // too small
        t.upsert(DeviceRecord::cpu(16384));
        t.upsert(DeviceRecord::cloud("openai"));
        let p = plan(
            &req(8000, PrivacyReq::Strict, true),
            &t,
            PolicyProfile::Balanced,
        );
        assert_eq!(p.device, DeviceId::Cpu);
        assert_eq!(p.rationale, RationaleCode::PrivacyLocalOnly);
    }
}
