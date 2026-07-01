//! Pressure Engine (HRA Task 7 / R5.3, R5.4).
//!
//! Ports the proven anti-thrash logic from `llm/orchestrator/gpu_watchdog.rs` (EMA + dwell +
//! hysteresis) behind the RA, expressed against the multi-band `Budget`. It produces a
//! `PressureLevel` and an ordered, non-disruptive-first `Remedy` recommendation. It never itself
//! interrupts a foreground stream — only `Emergency` permits that, and the runtime routes it
//! through the Foreground Guard with a streaming checkpoint (Property 4 / Property 10).

use super::budget::Budget;

/// Three-sample-ish EMA over free memory (α=0.5 ≈ watchdog default).
#[derive(Debug, Clone)]
struct Ema {
    value: Option<f64>,
    alpha: f64,
}

impl Ema {
    fn new(alpha: f64) -> Self {
        Self { value: None, alpha }
    }
    fn update(&mut self, sample: u64) -> u64 {
        let s = sample as f64;
        let v = match self.value {
            None => s,
            Some(p) => self.alpha * s + (1.0 - self.alpha) * p,
        };
        self.value = Some(v);
        v as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureLevel {
    Normal,
    Yield,
    Emergency,
}

/// Ordered, non-disruptive-first remedy recommendation (R5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remedy {
    None,
    ReclaimIdleBackground,
    ShrinkBackgroundContext,
    DownshiftAtTurnBoundary,
    EvictBackgroundToRam,
    RouteNewWorkElsewhere,
    /// Emergency only — foreground-protecting action with checkpoint+resume.
    EmergencyCheckpoint,
}

#[derive(Debug, Clone)]
pub struct PressureEngine {
    ema: Ema,
    /// Consecutive samples in the yield band (dwell debounce).
    yield_dwell: u32,
    /// Consecutive samples in the emergency band.
    emergency_dwell: u32,
    /// Required dwell counts before acting.
    yield_dwell_required: u32,
    emergency_dwell_required: u32,
    last_level: PressureLevel,
}

impl Default for PressureEngine {
    fn default() -> Self {
        Self::new(5, 2)
    }
}

impl PressureEngine {
    pub fn new(yield_dwell_required: u32, emergency_dwell_required: u32) -> Self {
        Self {
            ema: Ema::new(0.5),
            yield_dwell: 0,
            emergency_dwell: 0,
            yield_dwell_required,
            emergency_dwell_required,
            last_level: PressureLevel::Normal,
        }
    }

    pub fn level(&self) -> PressureLevel {
        self.last_level
    }

    /// Feed a raw free-memory sample. Returns the debounced level + recommended remedy.
    /// Hysteresis: exit Yield requires free above `soft + hysteresis`.
    pub fn observe(&mut self, raw_free_mb: u64, budget: &Budget, hysteresis_mb: u64) -> (PressureLevel, Remedy) {
        let free = self.ema.update(raw_free_mb);

        // Emergency overlay (debounced).
        if budget.in_emergency(free) {
            self.emergency_dwell += 1;
            self.yield_dwell = 0;
            if self.emergency_dwell >= self.emergency_dwell_required {
                self.last_level = PressureLevel::Emergency;
                return (PressureLevel::Emergency, Remedy::EmergencyCheckpoint);
            }
            // Not yet sustained → treat as yield in the meantime.
            self.last_level = PressureLevel::Yield;
            return (PressureLevel::Yield, Remedy::ReclaimIdleBackground);
        }
        self.emergency_dwell = 0;

        // Yield band (debounced) with hysteresis on exit.
        let in_yield = budget.in_soft(free);
        let exited = free > budget.soft_mb.saturating_add(hysteresis_mb);

        if in_yield {
            self.yield_dwell += 1;
            if self.yield_dwell >= self.yield_dwell_required {
                self.last_level = PressureLevel::Yield;
                return (PressureLevel::Yield, Self::yield_remedy(self.yield_dwell));
            }
            // building dwell, stay at last (no premature action)
            return (self.last_level, Remedy::None);
        }

        if exited {
            self.yield_dwell = 0;
            self.last_level = PressureLevel::Normal;
            return (PressureLevel::Normal, Remedy::None);
        }

        // In the hysteresis deadband: hold previous level, no new remedy.
        (self.last_level, Remedy::None)
    }

    /// Escalate remedy as dwell persists, staying non-disruptive until forced.
    fn yield_remedy(dwell: u32) -> Remedy {
        match dwell {
            d if d < 8 => Remedy::ReclaimIdleBackground,
            d if d < 12 => Remedy::ShrinkBackgroundContext,
            d if d < 16 => Remedy::EvictBackgroundToRam,
            _ => Remedy::RouteNewWorkElsewhere,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::budget::{BandPolicy, Budget};
    use super::*;

    fn budget() -> Budget {
        Budget::derive(12_288, 512, BandPolicy::default())
    }

    #[test]
    fn single_dip_does_not_trigger_yield() {
        let mut p = PressureEngine::new(5, 2);
        let b = budget();
        // one low sample then recovery — dwell never reached.
        let (lvl, _) = p.observe(b.soft_mb.saturating_sub(10), &b, 256);
        assert_ne!(lvl, PressureLevel::Yield);
        let (lvl2, _) = p.observe(b.soft_mb + 5000, &b, 256);
        assert_eq!(lvl2, PressureLevel::Normal);
    }

    #[test]
    fn sustained_pressure_yields_with_nondisruptive_first() {
        let mut p = PressureEngine::new(3, 2);
        let b = budget();
        let mut last = (PressureLevel::Normal, Remedy::None);
        for _ in 0..4 {
            last = p.observe(b.soft_mb.saturating_sub(50), &b, 256);
        }
        assert_eq!(last.0, PressureLevel::Yield);
        assert_eq!(last.1, Remedy::ReclaimIdleBackground); // least disruptive first
    }

    #[test]
    fn emergency_only_after_dwell_and_recommends_checkpoint() {
        let mut p = PressureEngine::new(5, 2);
        let b = budget();
        let first = p.observe(b.emergency_mb.saturating_sub(10), &b, 256);
        assert_eq!(first.0, PressureLevel::Yield); // not yet sustained
        let second = p.observe(b.emergency_mb.saturating_sub(10), &b, 256);
        assert_eq!(second.0, PressureLevel::Emergency);
        assert_eq!(second.1, Remedy::EmergencyCheckpoint);
    }

    #[test]
    fn hysteresis_prevents_flapping_at_boundary() {
        let mut p = PressureEngine::new(2, 2);
        let b = budget();
        // drive into yield
        p.observe(b.soft_mb - 50, &b, 256);
        let (lvl, _) = p.observe(b.soft_mb - 50, &b, 256);
        assert_eq!(lvl, PressureLevel::Yield);
        // small bump just above soft but within hysteresis → stay Yield (no flap)
        let (lvl2, _) = p.observe(b.soft_mb + 100, &b, 256);
        assert_eq!(lvl2, PressureLevel::Yield);
        // clear hysteresis band → Normal
        let (lvl3, _) = p.observe(b.soft_mb + 5000, &b, 256);
        assert_eq!(lvl3, PressureLevel::Normal);
    }
}
