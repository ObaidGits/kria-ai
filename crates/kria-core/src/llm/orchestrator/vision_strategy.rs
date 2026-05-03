//! Multi-tiered vision degradation strategy.
//!
//! Replaces the binary `enable_vision: bool` with a graduated fallback that
//! never fully blinds the LLM. Instead of dropping `--mmproj` entirely at
//! low VRAM, the system downscales input images or runs vision on CPU.
//!
//! ## Tiers
//! - **FullGpu**: ngl >= vision_min_ngl. Full resolution, roi_hybrid allowed.
//! - **ReducedGpu**: 0 < ngl < vision_min_ngl. GPU vision but capped at 512×512.
//! - **CpuVision**: ngl == 0 with sufficient RAM. mmproj loaded in system RAM.
//! - **Disabled**: Insufficient RAM for even CPU vision. OCR fallback only.

use crate::config::ModelProfile;

/// Vision capability tier — ordered from best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VisionMode {
    /// Full resolution, GPU-accelerated vision. roi_hybrid mode allowed.
    FullGpu,
    /// GPU vision but input images capped at 512×512 to limit KV cache cost.
    ReducedGpu,
    /// LLM runs on CPU (ngl=0) but mmproj is loaded in system RAM.
    /// Vision works but is slow (~3-8s per image on modern CPUs).
    CpuVision,
    /// No vision capability. Falls back to OCR text extraction.
    Disabled,
}

impl VisionMode {
    /// Whether the LLM can process image inputs at all.
    pub fn has_vision(&self) -> bool {
        !matches!(self, Self::Disabled)
    }

    /// Whether `--mmproj` should be passed to llama-server.
    pub fn load_mmproj(&self) -> bool {
        !matches!(self, Self::Disabled)
    }

    /// Maximum image dimension (width or height) for this tier.
    /// 0 = no limit (full resolution).
    pub fn max_image_dimension(&self) -> u32 {
        match self {
            Self::FullGpu => 0,       // No limit
            Self::ReducedGpu => 512,  // Cap to 512×512
            Self::CpuVision => 256,   // Cap to 256×256 (CPU is slow)
            Self::Disabled => 0,      // N/A
        }
    }

    /// Stable string slug for tracing/serialization.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FullGpu => "full_gpu",
            Self::ReducedGpu => "reduced_gpu",
            Self::CpuVision => "cpu_vision",
            Self::Disabled => "disabled",
        }
    }

    /// Backward-compatible: does this mode satisfy `enable_vision == true`?
    pub fn is_enabled(&self) -> bool {
        self.has_vision()
    }
}

impl std::fmt::Display for VisionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Minimum free system RAM (MB) required to hold mmproj in CPU memory
/// for the CpuVision tier. Below this, vision is fully disabled.
const CPU_VISION_MIN_FREE_RAM_MB: u64 = 2048;

/// Determine the appropriate VisionMode given current hardware state.
///
/// # Arguments
/// * `profile` — model profile (has_vision_projector, vision_min_ngl)
/// * `ngl` — number of GPU layers the server will be spawned with
/// * `free_ram_mb` — current free system RAM (for CPU vision feasibility)
pub fn determine_vision_mode(
    profile: &ModelProfile,
    ngl: u32,
    free_ram_mb: u64,
) -> VisionMode {
    if !profile.has_vision_projector {
        return VisionMode::Disabled;
    }

    if ngl >= profile.vision_min_ngl {
        // Enough GPU capacity for full vision
        VisionMode::FullGpu
    } else if ngl > 0 {
        // Some GPU layers but not enough for full vision processing.
        // Cap resolution to reduce KV cache pressure.
        VisionMode::ReducedGpu
    } else {
        // ngl == 0: full CPU mode. Vision is still possible if we have
        // enough system RAM to hold the mmproj weights.
        if free_ram_mb >= CPU_VISION_MIN_FREE_RAM_MB {
            VisionMode::CpuVision
        } else {
            VisionMode::Disabled
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vision_profile() -> ModelProfile {
        ModelProfile {
            has_vision_projector: true,
            vision_min_ngl: 15,
            ..ModelProfile::default()
        }
    }

    fn text_only_profile() -> ModelProfile {
        ModelProfile {
            has_vision_projector: false,
            ..ModelProfile::default()
        }
    }

    #[test]
    fn full_gpu_when_enough_layers() {
        let mode = determine_vision_mode(&vision_profile(), 20, 8000);
        assert_eq!(mode, VisionMode::FullGpu);
        assert!(mode.has_vision());
        assert!(mode.load_mmproj());
        assert_eq!(mode.max_image_dimension(), 0);
    }

    #[test]
    fn reduced_gpu_below_threshold() {
        let mode = determine_vision_mode(&vision_profile(), 10, 8000);
        assert_eq!(mode, VisionMode::ReducedGpu);
        assert!(mode.has_vision());
        assert!(mode.load_mmproj());
        assert_eq!(mode.max_image_dimension(), 512);
    }

    #[test]
    fn cpu_vision_with_ram() {
        let mode = determine_vision_mode(&vision_profile(), 0, 4000);
        assert_eq!(mode, VisionMode::CpuVision);
        assert!(mode.has_vision());
        assert!(mode.load_mmproj());
        assert_eq!(mode.max_image_dimension(), 256);
    }

    #[test]
    fn disabled_without_ram() {
        let mode = determine_vision_mode(&vision_profile(), 0, 1000);
        assert_eq!(mode, VisionMode::Disabled);
        assert!(!mode.has_vision());
        assert!(!mode.load_mmproj());
    }

    #[test]
    fn disabled_no_projector() {
        let mode = determine_vision_mode(&text_only_profile(), 28, 16000);
        assert_eq!(mode, VisionMode::Disabled);
    }

    #[test]
    fn backward_compat_is_enabled() {
        assert!(VisionMode::FullGpu.is_enabled());
        assert!(VisionMode::ReducedGpu.is_enabled());
        assert!(VisionMode::CpuVision.is_enabled());
        assert!(!VisionMode::Disabled.is_enabled());
    }

    #[test]
    fn boundary_at_exact_threshold() {
        let mode = determine_vision_mode(&vision_profile(), 15, 8000);
        assert_eq!(mode, VisionMode::FullGpu);

        let mode = determine_vision_mode(&vision_profile(), 14, 8000);
        assert_eq!(mode, VisionMode::ReducedGpu);
    }

    // ── New tests: CpuVision state machine + ngl=0 regression ────────

    #[test]
    fn cpu_vision_boundary_at_exact_ram_threshold() {
        // Exactly 2048 MB RAM → should qualify for CpuVision
        let mode = determine_vision_mode(&vision_profile(), 0, 2048);
        assert_eq!(mode, VisionMode::CpuVision);

        // 1 MB below threshold → Disabled
        let mode = determine_vision_mode(&vision_profile(), 0, 2047);
        assert_eq!(mode, VisionMode::Disabled);
    }

    #[test]
    fn cpu_vision_must_load_mmproj() {
        // THIS IS THE CRITICAL REGRESSION TEST:
        // When ngl=0 and RAM is sufficient, CpuVision.load_mmproj() MUST
        // return true so --mmproj is passed to llama-server.
        let mode = determine_vision_mode(&vision_profile(), 0, 4000);
        assert_eq!(mode, VisionMode::CpuVision);
        assert!(mode.load_mmproj(),
            "BUG: CpuVision mode MUST load mmproj into system RAM. \
             Without --mmproj the LLM is blind even though CPU vision is possible.");
        assert!(mode.has_vision());
        assert!(mode.is_enabled());
    }

    #[test]
    fn all_modes_mmproj_contract() {
        // FullGpu, ReducedGpu, CpuVision → load_mmproj = true
        // Disabled → load_mmproj = false
        assert!(VisionMode::FullGpu.load_mmproj());
        assert!(VisionMode::ReducedGpu.load_mmproj());
        assert!(VisionMode::CpuVision.load_mmproj());
        assert!(!VisionMode::Disabled.load_mmproj());
    }

    #[test]
    fn max_image_dimension_contract() {
        assert_eq!(VisionMode::FullGpu.max_image_dimension(), 0);  // no limit
        assert_eq!(VisionMode::ReducedGpu.max_image_dimension(), 512);
        assert_eq!(VisionMode::CpuVision.max_image_dimension(), 256);
        assert_eq!(VisionMode::Disabled.max_image_dimension(), 0); // N/A
    }

    #[test]
    fn as_str_roundtrip() {
        assert_eq!(VisionMode::FullGpu.as_str(), "full_gpu");
        assert_eq!(VisionMode::ReducedGpu.as_str(), "reduced_gpu");
        assert_eq!(VisionMode::CpuVision.as_str(), "cpu_vision");
        assert_eq!(VisionMode::Disabled.as_str(), "disabled");
    }

    #[test]
    fn display_matches_as_str() {
        for mode in [VisionMode::FullGpu, VisionMode::ReducedGpu, VisionMode::CpuVision, VisionMode::Disabled] {
            assert_eq!(format!("{mode}"), mode.as_str());
        }
    }

    #[test]
    fn ordering_best_to_worst() {
        // PartialOrd: FullGpu < ReducedGpu < CpuVision < Disabled
        assert!(VisionMode::FullGpu < VisionMode::ReducedGpu);
        assert!(VisionMode::ReducedGpu < VisionMode::CpuVision);
        assert!(VisionMode::CpuVision < VisionMode::Disabled);
    }

    #[test]
    fn ngl_0_with_no_projector_is_disabled() {
        // Even with tons of RAM, a text-only model at ngl=0 is Disabled.
        let mode = determine_vision_mode(&text_only_profile(), 0, 32000);
        assert_eq!(mode, VisionMode::Disabled);
        assert!(!mode.load_mmproj());
    }
}
