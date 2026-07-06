//! HRA core types (control plane). Pure, serializable, no I/O, no LLM.
//!
//! These types are the shared vocabulary of the Resource Authority: requests, plans, leases,
//! devices, and the priority/residency enums. They are intentionally free of runtime state
//! (no `Instant`, no handles) so they can be unit-tested and, in future, serialized across a
//! transport boundary (HRA R23.3 distributed-readiness). Runtime-bearing structs live in the
//! engine modules that consume these.

use serde::{Deserialize, Serialize};

/// Identifies a compute target. `RemoteHost` is reserved for future multi-host execution
/// (HRA R23.3) and is not scheduled by the current local authority.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceId {
    Cpu,
    Gpu(u32),
    CloudPool(String),
    /// Reserved extension point — not placed by the local authority yet.
    RemoteHost(String, Box<DeviceId>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Cpu,
    Gpu,
    Cloud,
}

/// Where a model's weights physically live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Residency {
    Unloaded,
    DiskCold,
    RamWarm,
    VramHot,
    Cloud,
}

/// Priority ordering for admission and preemption. Higher = more important.
/// `RealtimeVoice` is preemption-protected during an active utterance (HRA R6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorityClass {
    Maintenance = 0,
    Batch = 1,
    InteractiveBg = 2,
    RealtimeVoice = 3,
    InteractiveFg = 4,
}

impl PriorityClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Maintenance => "maintenance",
            Self::Batch => "batch",
            Self::InteractiveBg => "interactive_bg",
            Self::RealtimeVoice => "realtime_voice",
            Self::InteractiveFg => "interactive_fg",
        }
    }
}

/// The subsystem requesting resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerId {
    Llm,
    Stt,
    Tts,
    Wake,
    Vision,
    Ocr,
    Image,
    Embed,
    Agent,
    Ext,
}

/// A resource quantity. `quota_rps` is meaningful for cloud pools only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Capacity {
    pub vram_mb: u64,
    pub ram_mb: u64,
    pub cpu_threads: u32,
    pub quota_rps: Option<u32>,
}

impl Capacity {
    pub fn vram(vram_mb: u64) -> Self {
        Self {
            vram_mb,
            ..Default::default()
        }
    }
}

/// Privacy constraint. `Strict` data must never egress to a cloud Device (HRA R23.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyReq {
    /// May use cloud.
    #[default]
    Standard,
    /// Local-only; never egress.
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerReq {
    #[default]
    Normal,
    /// Prefer low-power placement (battery).
    LowPower,
}

/// What a request needs from a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceNeed {
    pub vram_mb: u64,
    pub ram_mb: u64,
    pub cpu_threads: u32,
    /// Whether the consumer needs exclusive use of the device.
    pub exclusivity: bool,
    pub model_id: Option<String>,
    pub est_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraints {
    pub privacy: PrivacyReq,
    pub max_latency_ms: u32,
    pub cost_ceiling: Option<u32>,
    pub allow_cloud: bool,
    pub power: PowerReq,
}

impl Default for Constraints {
    fn default() -> Self {
        Self {
            privacy: PrivacyReq::Standard,
            max_latency_ms: 0, // 0 = no explicit ceiling
            cost_ceiling: None,
            allow_cloud: true,
            power: PowerReq::Normal,
        }
    }
}

/// Correlation id tying a request → plan → lease → events → journal (HRA R10.2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnId(pub String);

/// A resource request submitted to the authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRequest {
    pub consumer: ConsumerId,
    pub class: PriorityClass,
    pub need: ResourceNeed,
    pub constraints: Constraints,
    pub turn_id: TurnId,
}

/// Why the planner produced a given plan (drives explainability UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RationaleCode {
    FitsLocal,
    CoResident,
    EvictedBackground,
    ShrunkContext,
    Downshifted,
    FailoverCloud,
    FailOpenCpu,
    HardLimitBreach,
    PrivacyLocalOnly,
}

impl RationaleCode {
    /// Human-readable explanation for the UI (HRA R9.2).
    pub fn human(&self) -> &'static str {
        match self {
            Self::FitsLocal => "Ran locally — the model fit in available GPU memory.",
            Self::CoResident => {
                "Ran locally alongside other models — capacity allowed co-residency."
            }
            Self::EvictedBackground => "Freed a background model to make room.",
            Self::ShrunkContext => "Reduced context size to fit available memory.",
            Self::Downshifted => "Moved some layers to CPU to fit available memory.",
            Self::FailoverCloud => "Used cloud — local capacity was insufficient.",
            Self::FailOpenCpu => "Used CPU — no safe GPU plan was available in time.",
            Self::HardLimitBreach => {
                "Avoided an action that would have exceeded the safe memory limit."
            }
            Self::PrivacyLocalOnly => {
                "Kept local — data is marked privacy-strict and must not leave the device."
            }
        }
    }
}

/// A deterministic placement decision. `fallback_chain` is ordered safe alternatives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub device: DeviceId,
    pub residency: Residency,
    pub budget: Capacity,
    pub fallback_chain: Vec<Plan>,
    pub rationale: RationaleCode,
}

/// Monotonic authority epoch. Increments on every authority (Core) restart so pre-restart
/// leases are fenced off (HRA R21.1, split-brain protection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Epoch(pub u64);

impl Epoch {
    pub fn next(self) -> Self {
        Epoch(self.0 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_ordering_is_total_and_correct() {
        assert!(PriorityClass::InteractiveFg > PriorityClass::RealtimeVoice);
        assert!(PriorityClass::RealtimeVoice > PriorityClass::InteractiveBg);
        assert!(PriorityClass::InteractiveBg > PriorityClass::Batch);
        assert!(PriorityClass::Batch > PriorityClass::Maintenance);
    }

    #[test]
    fn epoch_increments_monotonically() {
        let e = Epoch(7);
        assert_eq!(e.next(), Epoch(8));
        assert!(e.next() > e);
    }

    #[test]
    fn types_round_trip_serde() {
        let req = ResourceRequest {
            consumer: ConsumerId::Llm,
            class: PriorityClass::InteractiveFg,
            need: ResourceNeed {
                vram_mb: 4096,
                ram_mb: 2048,
                cpu_threads: 4,
                exclusivity: true,
                model_id: Some("qwen3-vl-4b".into()),
                est_ms: 1200,
            },
            constraints: Constraints::default(),
            turn_id: TurnId("t-1".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ResourceRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn rationale_has_human_text() {
        assert!(!RationaleCode::FailoverCloud.human().is_empty());
        assert!(RationaleCode::PrivacyLocalOnly.human().contains("local"));
    }
}
