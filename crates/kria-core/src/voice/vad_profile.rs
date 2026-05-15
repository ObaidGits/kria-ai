//! VAD environment profiles per `ENHANCED_STT.md` §13 (fixed tables, no auto-switch).

use serde::{Deserialize, Serialize};

/// VAD profile configuration (§13).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VadProfileConfig {
    /// Active profile: "quiet" | "normal" | "noisy"
    pub active_profile: String,
}

impl Default for VadProfileConfig {
    fn default() -> Self {
        Self {
            active_profile: "normal".to_string(),
        }
    }
}

impl VadProfileConfig {
    pub fn new(profile: &str) -> Self {
        Self {
            active_profile: normalize_label(profile),
        }
    }
    
    pub fn profile(&self) -> &str {
        &self.active_profile
    }
}

/// Normalise user/config input to `quiet` | `normal` | `noisy`.
pub fn normalize_label(s: &str) -> String {
    match s.to_lowercase().as_str() {
        "quiet" => "quiet".into(),
        "noisy" => "noisy".into(),
        _ => "normal".into(),
    }
}

/// Silero speech-probability threshold for the given profile.
pub fn silero_threshold(profile_label: &str) -> f32 {
    match normalize_label(profile_label).as_str() {
        "quiet" => 0.55,
        "noisy" => 0.45,
        _ => 0.50,
    }
}

/// `(tail_padding_ms, min_speech_ms)` from §13.
pub fn tail_and_min_speech_ms(profile_label: &str) -> (u64, u64) {
    match normalize_label(profile_label).as_str() {
        "quiet" => (280, 120),
        "noisy" => (550, 200),
        _ => (400, 150),
    }
}

/// RMS thresholds for the v2 energy-based endpoint detector in `pipeline.rs`:
/// `(start_rms, end_rms)`.
pub fn v2_capture_rms_thresholds(profile_label: &str) -> (f32, f32) {
    match normalize_label(profile_label).as_str() {
        "quiet" => (0.0025, 0.003),
        "noisy" => (0.0015, 0.003),
        _ => (0.002, 0.003),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_label_handles_all_profiles() {
        assert_eq!(normalize_label("quiet"), "quiet");
        assert_eq!(normalize_label("QUIET"), "quiet");
        assert_eq!(normalize_label("normal"), "normal");
        assert_eq!(normalize_label("NORMAL"), "normal");
        assert_eq!(normalize_label("noisy"), "noisy");
        assert_eq!(normalize_label("NOISY"), "noisy");
        assert_eq!(normalize_label("unknown"), "normal");
        assert_eq!(normalize_label(""), "normal");
    }

    #[test]
    fn silero_threshold_returns_correct_values() {
        assert_eq!(silero_threshold("quiet"), 0.55);
        assert_eq!(silero_threshold("normal"), 0.50);
        assert_eq!(silero_threshold("noisy"), 0.45);
        assert_eq!(silero_threshold("unknown"), 0.50);
    }

    #[test]
    fn tail_and_min_speech_ms_returns_correct_values() {
        assert_eq!(tail_and_min_speech_ms("quiet"), (280, 120));
        assert_eq!(tail_and_min_speech_ms("normal"), (400, 150));
        assert_eq!(tail_and_min_speech_ms("noisy"), (550, 200));
        assert_eq!(tail_and_min_speech_ms("unknown"), (400, 150));
    }

    #[test]
    fn v2_capture_rms_thresholds_returns_correct_values() {
        assert_eq!(v2_capture_rms_thresholds("quiet"), (0.0025, 0.003));
        assert_eq!(v2_capture_rms_thresholds("normal"), (0.002, 0.003));
        assert_eq!(v2_capture_rms_thresholds("noisy"), (0.0015, 0.003));
        assert_eq!(v2_capture_rms_thresholds("unknown"), (0.002, 0.003));
    }

    #[test]
    fn vad_profile_config_default() {
        let config = VadProfileConfig::default();
        assert_eq!(config.active_profile, "normal");
    }

    #[test]
    fn vad_profile_config_new_normalizes() {
        let config = VadProfileConfig::new("QUIET");
        assert_eq!(config.active_profile, "quiet");
        
        let config = VadProfileConfig::new("unknown");
        assert_eq!(config.active_profile, "normal");
    }

    #[test]
    fn vad_profile_config_profile_getter() {
        let config = VadProfileConfig::new("noisy");
        assert_eq!(config.profile(), "noisy");
    }

    #[test]
    fn vad_profile_config_serialization() {
        let config = VadProfileConfig::new("quiet");
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"active_profile\":\"quiet\""));
        
        let deserialized: VadProfileConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }
}
