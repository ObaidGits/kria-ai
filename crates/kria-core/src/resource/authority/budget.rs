//! Multi-band memory budget (HRA Task 45 / R27).
//!
//! Soft/Hard/Emergency limits are DERIVED from the existing capacity + safety values — there is
//! exactly one accounting of memory per device (Property 18, no duplicate counters). The bands are
//! a *view*, not new state:
//! - Soft Limit:      free below this → begin non-disruptive remedies (maps to Pressure "yield").
//! - Hard Limit:      free below this → refuse new admissions / shed (Scheduler admission gate).
//! - Emergency Limit: free below this → allow foreground-protecting emergency action (maps to
//!   Pressure "critical").

use serde::{Deserialize, Serialize};

/// Derived band thresholds expressed as "minimum free MB" trip points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    /// Begin reclaiming when free VRAM/RAM drops below this.
    pub soft_mb: u64,
    /// Refuse new admissions when free drops below this.
    pub hard_mb: u64,
    /// Emergency action permitted when free drops below this.
    pub emergency_mb: u64,
}

/// Percentage knobs for band derivation (of total). Defaults chosen to align with the existing
/// dynamic threshold profile (yield ~10%, emergency ~3%) plus an admission hard floor between them.
#[derive(Debug, Clone, Copy)]
pub struct BandPolicy {
    pub soft_pct: f64,
    pub hard_pct: f64,
    pub emergency_pct: f64,
}

impl Default for BandPolicy {
    fn default() -> Self {
        Self {
            soft_pct: 0.10,
            hard_pct: 0.06,
            emergency_pct: 0.03,
        }
    }
}

impl Budget {
    /// Derive bands from a device's `total_mb` and reserved `safety_mb`. Single source: the trip
    /// points are computed from total; safety_mb raises the floor so Hard never sits below the
    /// reserved safety margin.
    ///
    /// Invariant maintained: `emergency_mb <= hard_mb <= soft_mb`.
    pub fn derive(total_mb: u64, safety_mb: u64, policy: BandPolicy) -> Self {
        let pct = |p: f64| (total_mb as f64 * p) as u64;
        let emergency = pct(policy.emergency_pct).max(64);
        let hard = pct(policy.hard_pct).max(safety_mb).max(emergency);
        let soft = pct(policy.soft_pct).max(hard);
        Self {
            soft_mb: soft,
            hard_mb: hard,
            emergency_mb: emergency,
        }
    }

    /// True when `free_mb` is at/under the soft band → start non-disruptive remedies.
    pub fn in_soft(&self, free_mb: u64) -> bool {
        free_mb <= self.soft_mb
    }
    /// True when admitting `need_mb` would drop free below the hard limit → refuse admission.
    pub fn admission_breaches_hard(&self, free_mb: u64, need_mb: u64) -> bool {
        free_mb.saturating_sub(need_mb) < self.hard_mb
    }
    /// True when `free_mb` is at/under the emergency band → emergency action allowed.
    pub fn in_emergency(&self, free_mb: u64) -> bool {
        free_mb <= self.emergency_mb
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_are_ordered() {
        let b = Budget::derive(24_576, 512, BandPolicy::default());
        assert!(b.emergency_mb <= b.hard_mb, "{b:?}");
        assert!(b.hard_mb <= b.soft_mb, "{b:?}");
    }

    #[test]
    fn safety_margin_raises_hard_floor() {
        // Tiny total so percentages are small; safety_mb should dominate hard.
        let b = Budget::derive(4096, 1024, BandPolicy::default());
        assert!(b.hard_mb >= 1024);
        assert!(b.soft_mb >= b.hard_mb);
    }

    #[test]
    fn admission_gate_refuses_when_breaching_hard() {
        let b = Budget::derive(12_288, 512, BandPolicy::default());
        // free comfortably above hard, small need → allowed
        assert!(!b.admission_breaches_hard(8000, 1000));
        // need pushes below hard → refused
        assert!(b.admission_breaches_hard(b.hard_mb + 100, 500));
    }

    #[test]
    fn soft_and_emergency_trip_points() {
        let b = Budget::derive(12_288, 512, BandPolicy::default());
        assert!(b.in_soft(b.soft_mb));
        assert!(!b.in_soft(b.soft_mb + 1));
        assert!(b.in_emergency(b.emergency_mb));
        assert!(!b.in_emergency(b.emergency_mb + 1));
    }

    #[test]
    fn single_accounting_no_negative() {
        let b = Budget::derive(6144, 256, BandPolicy::default());
        // saturating arithmetic: huge need never panics / underflows
        assert!(b.admission_breaches_hard(100, u64::MAX));
    }
}
