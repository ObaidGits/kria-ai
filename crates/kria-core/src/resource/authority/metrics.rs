//! SLO metrics (HRA Task 36 / R22.2).
//!
//! Low-cardinality counters + a tiny latency histogram. NO high-cardinality labels (turn_id lives
//! only in traces/journal, never here). Used to drive SLO dashboards/alerts.

#[derive(Debug, Clone, Default)]
pub struct Counters {
    pub admissions_granted: u64,
    pub admissions_busy: u64,
    pub admissions_shed: u64,
    pub preemptions: u64,
    pub swaps: u64,
    pub oom_events: u64,
    pub cloud_failovers: u64,
    pub foreground_interrupts_emergency: u64,
    pub foreground_interrupts_nonemergency: u64,
}

impl Counters {
    /// Whether the hard invariant holds: zero non-emergency foreground interrupts (A4/A16).
    pub fn foreground_invariant_ok(&self) -> bool {
        self.foreground_interrupts_nonemergency == 0
    }
}

/// Fixed-bucket latency histogram (ms). Bucket bounds are coarse → bounded cardinality.
#[derive(Debug, Clone)]
pub struct LatencyHistogram {
    bounds_ms: Vec<u32>,
    counts: Vec<u64>,
    count: u64,
    sum: u64,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new(&[50, 100, 250, 500, 1000, 2000, 4000, 8000, 20000])
    }
}

impl LatencyHistogram {
    pub fn new(bounds_ms: &[u32]) -> Self {
        Self {
            bounds_ms: bounds_ms.to_vec(),
            counts: vec![0; bounds_ms.len() + 1], // +1 for the overflow bucket
            count: 0,
            sum: 0,
        }
    }

    pub fn observe(&mut self, ms: u32) {
        let idx = self
            .bounds_ms
            .iter()
            .position(|b| ms <= *b)
            .unwrap_or(self.bounds_ms.len());
        self.counts[idx] += 1;
        self.count += 1;
        self.sum += ms as u64;
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn mean_ms(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum as f64 / self.count as f64
        }
    }

    /// Approximate percentile (upper bucket bound). p in 0..=100.
    pub fn p_estimate_ms(&self, p: u8) -> u32 {
        if self.count == 0 {
            return 0;
        }
        let target = (self.count as f64 * (p as f64 / 100.0)).ceil() as u64;
        let mut cum = 0;
        for (i, c) in self.counts.iter().enumerate() {
            cum += c;
            if cum >= target {
                return self.bounds_ms.get(i).copied().unwrap_or(u32::MAX);
            }
        }
        u32::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_invariant_tracks_nonemergency() {
        let mut c = Counters::default();
        assert!(c.foreground_invariant_ok());
        c.foreground_interrupts_emergency += 1; // emergency ok
        assert!(c.foreground_invariant_ok());
        c.foreground_interrupts_nonemergency += 1; // violates
        assert!(!c.foreground_invariant_ok());
    }

    #[test]
    fn histogram_mean_and_percentile() {
        let mut h = LatencyHistogram::default();
        for _ in 0..90 {
            h.observe(40);
        }
        for _ in 0..10 {
            h.observe(1500);
        }
        assert_eq!(h.count(), 100);
        assert!(h.mean_ms() > 40.0 && h.mean_ms() < 200.0);
        // p50 should land in the small bucket, p99 in the larger one.
        assert!(h.p_estimate_ms(50) <= 50);
        assert!(h.p_estimate_ms(99) >= 1000);
    }
}
