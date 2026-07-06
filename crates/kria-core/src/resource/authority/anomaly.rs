//! Anomaly detectors (HRA Task 18 / R10.3).
//!
//! Deterministic detectors that emit a root-cause hypothesis with evidence. They run in the Health
//! Monitor over telemetry windows + journal data. Each detector requires a dwell to avoid
//! false positives.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnomalyKind {
    CpuSpike,
    GpuSpike,
    VramLeak,
    RamLeak,
    Starvation,
    HungModel,
    ThermalThrottle,
    InfiniteRetry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anomaly {
    pub kind: AnomalyKind,
    pub hypothesis: String,
    /// Evidence: the offending entity (process/consumer) when identifiable.
    pub suspect: Option<String>,
}

/// Spike detector: value above `threshold` for `dwell` consecutive samples → spike.
pub fn detect_spike(
    kind: AnomalyKind,
    samples: &[u32],
    threshold: u32,
    dwell: usize,
    suspect: Option<String>,
) -> Option<Anomaly> {
    if samples.len() < dwell {
        return None;
    }
    let tail = &samples[samples.len() - dwell..];
    if tail.iter().all(|v| *v >= threshold) {
        Some(Anomaly {
            kind,
            hypothesis: format!(
                "sustained ≥{threshold} for {dwell} samples (latest {})",
                tail.last().copied().unwrap_or(0)
            ),
            suspect,
        })
    } else {
        None
    }
}

/// Leak detector: a monotonic non-reclaimed increase in used memory across idle windows.
/// `used_mb` are samples taken during idle; a strictly increasing run ≥ `min_run` flags a leak.
pub fn detect_leak(
    kind: AnomalyKind,
    used_mb: &[u64],
    min_run: usize,
    suspect: Option<String>,
) -> Option<Anomaly> {
    if used_mb.len() < min_run {
        return None;
    }
    let strictly_increasing = used_mb.windows(2).all(|w| w[1] > w[0]);
    if strictly_increasing {
        let growth = used_mb.last().unwrap() - used_mb.first().unwrap();
        Some(Anomaly {
            kind,
            hypothesis: format!("memory grew {growth} MB monotonically while idle"),
            suspect,
        })
    } else {
        None
    }
}

/// Starvation: a request class waited longer than its SLA without admission.
pub fn detect_starvation(
    class: &str,
    waited_ms: u32,
    sla_ms: u32,
    blocking_holder: Option<String>,
) -> Option<Anomaly> {
    if waited_ms > sla_ms {
        Some(Anomaly {
            kind: AnomalyKind::Starvation,
            hypothesis: format!("{class} waited {waited_ms}ms > SLA {sla_ms}ms"),
            suspect: blocking_holder,
        })
    } else {
        None
    }
}

/// Hung model: a lease is active but telemetry shows no progress for `idle_samples` and health is
/// stale.
pub fn detect_hung(
    model: &str,
    progress_samples: &[u64],
    idle_samples: usize,
    health_stale: bool,
) -> Option<Anomaly> {
    if progress_samples.len() < idle_samples || !health_stale {
        return None;
    }
    let tail = &progress_samples[progress_samples.len() - idle_samples..];
    let no_progress = tail.windows(2).all(|w| w[1] == w[0]);
    if no_progress {
        Some(Anomaly {
            kind: AnomalyKind::HungModel,
            hypothesis: format!(
                "{model} active but no progress for {idle_samples} samples + stale health"
            ),
            suspect: Some(model.to_string()),
        })
    } else {
        None
    }
}

/// Infinite-retry: more than `max` attempts of the same op within the window.
pub fn detect_infinite_retry(op: &str, attempts: u32, max: u32) -> Option<Anomaly> {
    if attempts > max {
        Some(Anomaly {
            kind: AnomalyKind::InfiniteRetry,
            hypothesis: format!("{op} retried {attempts} times (> {max})"),
            suspect: Some(op.to_string()),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_spike_detected_after_dwell() {
        let a = detect_spike(
            AnomalyKind::GpuSpike,
            &[10, 95, 96, 97],
            90,
            3,
            Some("comfyui".into()),
        );
        assert!(a.is_some());
        assert_eq!(a.unwrap().suspect.as_deref(), Some("comfyui"));
    }

    #[test]
    fn spike_not_flagged_on_transient() {
        let a = detect_spike(AnomalyKind::CpuSpike, &[10, 99, 10, 10], 90, 3, None);
        assert!(a.is_none());
    }

    #[test]
    fn vram_leak_detected_on_monotonic_growth() {
        let a = detect_leak(
            AnomalyKind::VramLeak,
            &[1000, 1100, 1250, 1400],
            3,
            Some("llama".into()),
        );
        assert!(a.is_some());
    }

    #[test]
    fn leak_not_flagged_when_stable() {
        let a = detect_leak(AnomalyKind::RamLeak, &[1000, 1000, 1000], 3, None);
        assert!(a.is_none());
    }

    #[test]
    fn starvation_flags_over_sla_wait() {
        assert!(detect_starvation("batch", 9000, 8000, Some("fg".into())).is_some());
        assert!(detect_starvation("batch", 100, 8000, None).is_none());
    }

    #[test]
    fn hung_model_requires_stale_health_and_no_progress() {
        assert!(detect_hung("m", &[5, 5, 5], 3, true).is_some());
        assert!(detect_hung("m", &[5, 6, 7], 3, true).is_none());
        assert!(detect_hung("m", &[5, 5, 5], 3, false).is_none());
    }

    #[test]
    fn infinite_retry_flagged() {
        assert!(detect_infinite_retry("api_load", 11, 10).is_some());
        assert!(detect_infinite_retry("api_load", 3, 10).is_none());
    }
}
