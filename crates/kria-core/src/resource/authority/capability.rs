//! Capability Vector (HRA Task 24 / R18).
//!
//! Replaces single-axis `HardwareTier` for placement decisions with a per-resource score vector.
//! The coarse tier remains a display label; the Planner reasons over this vector so a
//! CPU-heavy/no-GPU box and a GPU-heavy/low-RAM box (which can map to the same tier) get correct,
//! different plans. Pure + deterministic.

use serde::{Deserialize, Serialize};

use crate::platform::detect::HardwareInfo;

/// Per-resource capability scores in 0..=100. Higher = more capable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityVector {
    pub cpu: u8,
    pub gpu: u8,
    pub vram: u8,
    pub ram: u8,
    pub thermal: u8,
    pub power: u8,
}

impl CapabilityVector {
    /// Derive from detected hardware. Thermal/power default to "unknown-conservative" (50) until
    /// the Thermal & Power engine (Task 33) refines them at runtime.
    pub fn from_hardware(info: &HardwareInfo) -> Self {
        Self {
            cpu: score_cpu(info.cpu_cores),
            gpu: score_gpu(info.vram_mb),
            vram: score_vram(info.vram_mb.unwrap_or(0)),
            ram: score_ram(info.total_ram_mb),
            thermal: 50,
            power: 50,
        }
    }

    /// Whether this machine can run GPU inference at a meaningful scale.
    pub fn gpu_capable(&self) -> bool {
        self.gpu >= 25 && self.vram >= 25
    }

    /// Whether parallel co-residency (LLM + image) is plausible from capability alone.
    pub fn supports_co_residency(&self) -> bool {
        self.vram >= 70 && self.ram >= 50
    }

    /// A 0..=100 overall score (max of the compute axes, gated by memory) for coarse ranking.
    pub fn overall(&self) -> u8 {
        let compute = self.cpu.max(self.gpu);
        let memory = self.vram.max(self.ram);
        ((compute as u16 + memory as u16) / 2) as u8
    }
}

fn score_cpu(cores: usize) -> u8 {
    // 1 core → ~12, 4 → ~50, 8 → ~75, 16+ → ~95+
    let c = cores.min(32) as f64;
    ((c / 16.0).min(1.0) * 90.0 + 10.0).min(100.0) as u8
}

fn score_gpu(vram_mb: Option<u64>) -> u8 {
    match vram_mb {
        None | Some(0) => 0,
        Some(v) => score_vram(v),
    }
}

fn score_vram(vram_mb: u64) -> u8 {
    // 2 GB → ~16, 6 GB → ~50, 12 GB → ~75, 24 GB → ~95
    let gb = (vram_mb as f64) / 1024.0;
    if gb <= 0.0 {
        return 0;
    }
    ((gb / 24.0).min(1.0) * 92.0 + 8.0).min(100.0) as u8
}

fn score_ram(ram_mb: u64) -> u8 {
    // 4 GB → ~20, 16 GB → ~60, 32 GB → ~78, 64 GB+ → ~95
    let gb = (ram_mb as f64) / 1024.0;
    ((gb / 64.0).min(1.0) * 90.0 + 6.0).min(100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::detect::{HardwareInfo, HardwareTier, Os};
    use crate::platform::vram::ImageTier;

    fn info(cores: usize, ram_mb: u64, vram_mb: Option<u64>) -> HardwareInfo {
        HardwareInfo {
            os: Os::Linux,
            tier: HardwareTier::Standard,
            cpu_cores: cores,
            total_ram_mb: ram_mb,
            vram_mb,
            gpu_name: None,
            package_manager: None,
            hostname: "t".into(),
            vram_free_mb: 0,
            image_tier: ImageTier::default(),
        }
    }

    #[test]
    fn cpu_heavy_no_gpu_scores_high_cpu_zero_gpu() {
        let v = CapabilityVector::from_hardware(&info(16, 65536, None));
        assert!(v.cpu >= 80, "cpu={}", v.cpu);
        assert_eq!(v.gpu, 0);
        assert!(!v.gpu_capable());
    }

    #[test]
    fn gpu_heavy_low_ram_distinct_from_balanced() {
        // Same coarse tier could collapse these; capability vector keeps them distinct.
        let gpu_box = CapabilityVector::from_hardware(&info(8, 12288, Some(16384)));
        let ram_box = CapabilityVector::from_hardware(&info(8, 65536, Some(2048)));
        assert!(gpu_box.gpu > ram_box.gpu);
        assert!(ram_box.ram > gpu_box.ram);
        assert_ne!(gpu_box, ram_box);
    }

    #[test]
    fn co_residency_requires_big_vram_and_ram() {
        let big = CapabilityVector::from_hardware(&info(16, 65536, Some(24576)));
        let small = CapabilityVector::from_hardware(&info(8, 16384, Some(6144)));
        assert!(big.supports_co_residency());
        assert!(!small.supports_co_residency());
    }

    #[test]
    fn scores_are_bounded() {
        let v = CapabilityVector::from_hardware(&info(128, 1_000_000, Some(1_000_000)));
        for s in [v.cpu, v.gpu, v.vram, v.ram, v.thermal, v.power] {
            assert!(s <= 100);
        }
    }
}
