//! Thermal & Power Policy Engine (HRA Task 33 / R17).
//!
//! Deterministic mapping from thermal/power state to a `PolicyProfile`, a GPU duty-cycle budget,
//! and a prewarm veto. Degrades safely to a conservative "thermal-unknown" profile when sensors are
//! absent (R17.3) — never blocks on missing sensors.

use super::planner::PolicyProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    Ac,
    Battery { percent: u8 },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThermalState {
    /// Hottest relevant sensor in Celsius, if known.
    pub temp_c: Option<f32>,
    /// Throttle threshold for this device in Celsius (driver/junction limit).
    pub throttle_c: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PowerDecision {
    pub profile: PolicyProfile,
    /// GPU duty-cycle budget 0..=100 (% of time GPU may be driven hard).
    pub gpu_duty_pct: u8,
    /// Whether speculative prewarm should be vetoed right now.
    pub veto_prewarm: bool,
}

/// Decide the active policy from power + thermal. Pure.
pub fn decide(power: PowerSource, thermal: ThermalState) -> PowerDecision {
    // Thermal headroom dominates: if we're close to throttle, cap duty regardless of power.
    let thermal_capped = match thermal.temp_c {
        Some(t) => t >= thermal.throttle_c - 5.0,
        None => false, // unknown sensors → don't assume hot, but stay conservative below
    };

    match power {
        PowerSource::Battery { percent } if percent <= 20 => PowerDecision {
            profile: PolicyProfile::BatterySaver,
            gpu_duty_pct: 30,
            veto_prewarm: true,
        },
        PowerSource::Battery { .. } => PowerDecision {
            profile: PolicyProfile::BatterySaver,
            gpu_duty_pct: if thermal_capped { 40 } else { 60 },
            veto_prewarm: thermal_capped,
        },
        PowerSource::Ac => {
            if thermal_capped {
                PowerDecision {
                    profile: PolicyProfile::ThermalCapped,
                    gpu_duty_pct: 60,
                    veto_prewarm: true,
                }
            } else {
                PowerDecision {
                    profile: PolicyProfile::Performance,
                    gpu_duty_pct: 100,
                    veto_prewarm: false,
                }
            }
        }
        PowerSource::Unknown => {
            // Sensor-absent desktop: conservative Balanced, moderate duty, no veto. Never blocks.
            PowerDecision {
                profile: if thermal_capped {
                    PolicyProfile::ThermalCapped
                } else {
                    PolicyProfile::Balanced
                },
                gpu_duty_pct: if thermal_capped { 70 } else { 90 },
                veto_prewarm: thermal_capped,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cool() -> ThermalState {
        ThermalState {
            temp_c: Some(60.0),
            throttle_c: 90.0,
        }
    }
    fn hot() -> ThermalState {
        ThermalState {
            temp_c: Some(87.0),
            throttle_c: 90.0,
        }
    }
    fn nosensors() -> ThermalState {
        ThermalState {
            temp_c: None,
            throttle_c: 90.0,
        }
    }

    #[test]
    fn ac_cool_runs_performance_full_duty() {
        let d = decide(PowerSource::Ac, cool());
        assert_eq!(d.profile, PolicyProfile::Performance);
        assert_eq!(d.gpu_duty_pct, 100);
        assert!(!d.veto_prewarm);
    }

    #[test]
    fn ac_hot_caps_thermally() {
        let d = decide(PowerSource::Ac, hot());
        assert_eq!(d.profile, PolicyProfile::ThermalCapped);
        assert!(d.gpu_duty_pct < 100);
        assert!(d.veto_prewarm);
    }

    #[test]
    fn low_battery_saves_power_and_vetoes_prewarm() {
        let d = decide(PowerSource::Battery { percent: 15 }, cool());
        assert_eq!(d.profile, PolicyProfile::BatterySaver);
        assert!(d.veto_prewarm);
        assert!(d.gpu_duty_pct <= 30);
    }

    #[test]
    fn unknown_sensors_degrade_safely_no_block() {
        let d = decide(PowerSource::Unknown, nosensors());
        assert_eq!(d.profile, PolicyProfile::Balanced);
        assert!(!d.veto_prewarm);
        assert!(d.gpu_duty_pct >= 70);
    }
}
