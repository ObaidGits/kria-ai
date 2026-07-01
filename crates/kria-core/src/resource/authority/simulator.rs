//! Resource Simulator (HRA Task 43 / R25).
//!
//! Pure, deterministic pre-commit estimator. Before the Scheduler commits a disruptive action
//! (unload / swap / evict / image transition / cloud failover) it calls `simulate()` to estimate
//! VRAM/RAM/latency impact, disruption level, and risk. The estimate is journaled with the
//! decision; a predicted hard-limit breach forces a fallback instead of commit (Property 16).
//!
//! No I/O, no LLM. Estimates are conservative (bias toward higher disruption/risk) and are
//! calibrated against the Benchmark Framework (Task 48) over time.

use serde::{Deserialize, Serialize};

use super::budget::Budget;

/// A minimal device state slice fed to the simulator (taken from the telemetry snapshot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimDeviceState {
    pub free_vram_mb: u64,
    pub total_vram_mb: u64,
    pub free_ram_mb: u64,
    pub budget: Budget,
}

/// Actions the simulator can estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimAction {
    /// Unload a model currently using `model_vram_mb` of VRAM.
    Unload { model_vram_mb: u64 },
    /// Swap an LLM from `from_vram_mb` to `to_vram_mb` (ngl/ctx change).
    Swap { from_vram_mb: u64, to_vram_mb: u64 },
    /// Evict a model from VRAM to RAM (frees VRAM, costs RAM).
    EvictToRam { model_vram_mb: u64 },
    /// Image-generation transition needing `required_vram_mb` free for the backend.
    ImageTransition { required_vram_mb: u64 },
    /// Route a request to a cloud pool (no local VRAM impact, network latency).
    CloudFailover { rtt_ms: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disruption {
    None,
    Background,
    Interactive,
    Foreground,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Med,
    High,
}

/// The simulation result, journaled with the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Estimate {
    /// Predicted change in free VRAM (positive = more free after action).
    pub d_vram_mb: i64,
    /// Predicted change in free RAM (positive = more free after action).
    pub d_ram_mb: i64,
    pub est_latency_ms: u32,
    pub disruption: Disruption,
    pub risk: RiskLevel,
    /// Convenience: predicted free VRAM after the action.
    pub projected_free_vram_mb: u64,
    /// True when the projected free VRAM would sit below the device hard limit.
    pub breaches_hard_limit: bool,
}

// Conservative latency constants (ms). Restart-class ops dominate; calibrated later.
const LAT_UNLOAD: u32 = 400;
const LAT_SWAP_RESTART: u32 = 2500;
const LAT_EVICT: u32 = 1800;
const LAT_IMAGE_BARRIER: u32 = 3000;

/// Deterministically estimate the impact of `action` given `state`.
pub fn simulate(action: &SimAction, state: &SimDeviceState) -> Estimate {
    let (d_vram, d_ram, latency, disruption) = match action {
        SimAction::Unload { model_vram_mb } => {
            (*model_vram_mb as i64, 0, LAT_UNLOAD, Disruption::Background)
        }
        SimAction::Swap {
            from_vram_mb,
            to_vram_mb,
        } => (
            *from_vram_mb as i64 - *to_vram_mb as i64,
            0,
            LAT_SWAP_RESTART,
            Disruption::Interactive,
        ),
        SimAction::EvictToRam { model_vram_mb } => (
            *model_vram_mb as i64,
            -(*model_vram_mb as i64),
            LAT_EVICT,
            Disruption::Interactive,
        ),
        SimAction::ImageTransition { required_vram_mb } => {
            // Disruption depends on whether the requirement already fits.
            let disruption = if state.free_vram_mb >= *required_vram_mb {
                Disruption::Background
            } else {
                Disruption::Interactive
            };
            (0, 0, LAT_IMAGE_BARRIER, disruption)
        }
        SimAction::CloudFailover { rtt_ms } => {
            (0, 0, (*rtt_ms).max(50), Disruption::None)
        }
    };

    let projected_free_vram = (state.free_vram_mb as i64 + d_vram).max(0) as u64;
    let breaches_hard_limit = match action {
        // Failover/unload free memory; they cannot breach the hard limit by themselves.
        SimAction::CloudFailover { .. } | SimAction::Unload { .. } | SimAction::EvictToRam { .. } => {
            false
        }
        SimAction::Swap { .. } => projected_free_vram < state.budget.hard_mb,
        SimAction::ImageTransition { required_vram_mb } => *required_vram_mb > state.free_vram_mb,
    };

    let risk = assess_risk(d_ram, projected_free_vram, state, breaches_hard_limit);

    Estimate {
        d_vram_mb: d_vram,
        d_ram_mb: d_ram,
        est_latency_ms: latency,
        disruption,
        risk,
        projected_free_vram_mb: projected_free_vram,
        breaches_hard_limit,
    }
}

fn assess_risk(
    d_ram: i64,
    projected_free_vram: u64,
    state: &SimDeviceState,
    breaches_hard_limit: bool,
) -> RiskLevel {
    if breaches_hard_limit || projected_free_vram <= state.budget.emergency_mb {
        return RiskLevel::High;
    }
    // RAM pressure from eviction.
    if d_ram < 0 && (state.free_ram_mb as i64 + d_ram) < 1024 {
        return RiskLevel::High;
    }
    if projected_free_vram <= state.budget.hard_mb {
        return RiskLevel::Med;
    }
    RiskLevel::Low
}

#[cfg(test)]
mod tests {
    use super::super::budget::{BandPolicy, Budget};
    use super::*;

    fn state(free_vram: u64, total: u64, free_ram: u64) -> SimDeviceState {
        SimDeviceState {
            free_vram_mb: free_vram,
            total_vram_mb: total,
            free_ram_mb: free_ram,
            budget: Budget::derive(total, 512, BandPolicy::default()),
        }
    }

    #[test]
    fn deterministic_same_inputs_same_output() {
        let s = state(3000, 12288, 16000);
        let a = SimAction::Swap {
            from_vram_mb: 4000,
            to_vram_mb: 2000,
        };
        assert_eq!(simulate(&a, &s), simulate(&a, &s));
    }

    #[test]
    fn unload_frees_vram_low_risk() {
        let s = state(1000, 12288, 16000);
        let e = simulate(&SimAction::Unload { model_vram_mb: 3000 }, &s);
        assert_eq!(e.d_vram_mb, 3000);
        assert_eq!(e.projected_free_vram_mb, 4000);
        assert!(!e.breaches_hard_limit);
    }

    #[test]
    fn swap_growing_footprint_breaches_hard_limit_high_risk() {
        // Swapping to a bigger footprint on a nearly-full device.
        let s = state(1200, 12288, 16000);
        let e = simulate(
            &SimAction::Swap {
                from_vram_mb: 1000,
                to_vram_mb: 1800,
            },
            &s,
        );
        // free goes 1200 + (1000-1800) = 400 → below hard
        assert_eq!(e.projected_free_vram_mb, 400);
        assert!(e.breaches_hard_limit);
        assert_eq!(e.risk, RiskLevel::High);
    }

    #[test]
    fn evict_with_low_ram_is_high_risk() {
        let s = state(500, 12288, 1200); // only 1.2 GB RAM free
        let e = simulate(&SimAction::EvictToRam { model_vram_mb: 3000 }, &s);
        assert!(e.d_vram_mb > 0 && e.d_ram_mb < 0);
        assert_eq!(e.risk, RiskLevel::High);
    }

    #[test]
    fn image_transition_insufficient_vram_breaches() {
        let s = state(2000, 12288, 16000);
        let e = simulate(&SimAction::ImageTransition { required_vram_mb: 4500 }, &s);
        assert!(e.breaches_hard_limit);
        assert_eq!(e.disruption, Disruption::Interactive);
    }

    #[test]
    fn cloud_failover_no_local_impact() {
        let s = state(100, 12288, 4000);
        let e = simulate(&SimAction::CloudFailover { rtt_ms: 250 }, &s);
        assert_eq!(e.d_vram_mb, 0);
        assert_eq!(e.disruption, Disruption::None);
        assert!(!e.breaches_hard_limit);
    }
}
